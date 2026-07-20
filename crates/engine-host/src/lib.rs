#![doc = "Python analytics engine lifecycle and IPC boundary."]

use std::fmt::{self, Display, Formatter};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use teratai_contracts::generated::engine_error::EngineError;
use teratai_contracts::generated::engine_handshake_request::EngineHandshakeRequest;
use teratai_contracts::generated::engine_handshake_response::EngineHandshakeResponse;

/// Version of the newline-delimited engine protocol implemented by this host.
pub const ENGINE_PROTOCOL_VERSION: &str = "1.0";
/// Python runtime line supported by the MVP analytics engine.
pub const PYTHON_VERSION_PREFIX: &str = "3.12.";
const HANDSHAKE_COMMAND: &str = "engine.handshake";
const ENGINE_MODULE: &str = "teratai_engine.sidecar";
const STDERR_LIMIT_BYTES: usize = 8 * 1024;

/// Runtime settings supplied by the desktop application when starting the engine.
#[derive(Debug, Clone)]
pub struct EngineHostConfig {
    /// Python 3.12 executable or launcher resolved by the application installation.
    pub python_executable: PathBuf,
    /// Directory containing the `teratai_engine` package.
    pub engine_module_root: PathBuf,
    /// Caller-generated correlation identifier for the startup exchange.
    pub request_id: String,
    /// Maximum time allowed for the engine to answer the startup handshake.
    pub handshake_timeout: Duration,
    /// Maximum time allowed for graceful shutdown before forced termination.
    pub shutdown_timeout: Duration,
    /// Protocol version required by the native host.
    pub expected_protocol_version: String,
    /// Engine package version required by the native host.
    pub expected_engine_version: String,
}

impl EngineHostConfig {
    /// Create a configuration without assuming an installation path.
    #[must_use]
    pub fn new(
        python_executable: impl Into<PathBuf>,
        engine_module_root: impl Into<PathBuf>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            python_executable: python_executable.into(),
            engine_module_root: engine_module_root.into(),
            request_id: request_id.into(),
            handshake_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(5),
            expected_protocol_version: ENGINE_PROTOCOL_VERSION.to_owned(),
            expected_engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

/// Failures that prevent the native host from trusting the engine process.
#[derive(Debug)]
pub enum EngineHostError {
    /// Host configuration is incomplete or unsafe.
    InvalidConfiguration(String),
    /// The Python process could not be launched.
    Spawn(io::Error),
    /// A process pipe could not be read or written.
    Io(io::Error),
    /// A wire message could not be serialized or decoded.
    Serialization(serde_json::Error),
    /// The engine did not answer before the configured deadline.
    HandshakeTimeout,
    /// The process exited or closed stdout before sending a response.
    HandshakeChannelClosed,
    /// The engine returned a canonical typed error.
    Remote(Box<EngineError>),
    /// A response did not correlate to the request.
    RequestMismatch { expected: String, actual: String },
    /// Native host and Python engine protocols differ.
    ProtocolMismatch { expected: String, actual: String },
    /// Native host and Python engine package versions differ.
    EngineVersionMismatch { expected: String, actual: String },
    /// The sidecar is not running on the required Python line.
    PythonVersionMismatch { expected: String, actual: String },
    /// The engine answered but failed its health check.
    Unhealthy { status: String },
    /// The engine omitted a required startup capability.
    MissingCapability(String),
    /// Graceful shutdown exceeded the configured deadline and required termination.
    ShutdownTimeout,
}

impl Display for EngineHostError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(detail) => {
                write!(formatter, "invalid engine host configuration: {detail}")
            }
            Self::Spawn(error) => write!(formatter, "failed to start Python engine: {error}"),
            Self::Io(error) => write!(formatter, "engine pipe failed: {error}"),
            Self::Serialization(error) => {
                write!(formatter, "engine protocol message is invalid: {error}")
            }
            Self::HandshakeTimeout => formatter.write_str("engine handshake timed out"),
            Self::HandshakeChannelClosed => {
                formatter.write_str("engine closed before completing handshake")
            }
            Self::Remote(error) => write!(
                formatter,
                "engine rejected handshake [{}]: {}",
                error.code, error.message
            ),
            Self::RequestMismatch { expected, actual } => write!(
                formatter,
                "engine request mismatch: expected {expected}, received {actual}"
            ),
            Self::ProtocolMismatch { expected, actual } => write!(
                formatter,
                "engine protocol mismatch: expected {expected}, received {actual}"
            ),
            Self::EngineVersionMismatch { expected, actual } => write!(
                formatter,
                "engine version mismatch: expected {expected}, received {actual}"
            ),
            Self::PythonVersionMismatch { expected, actual } => write!(
                formatter,
                "Python version mismatch: expected {expected}, received {actual}"
            ),
            Self::Unhealthy { status } => {
                write!(formatter, "engine health check failed with status {status}")
            }
            Self::MissingCapability(capability) => {
                write!(formatter, "engine capability is missing: {capability}")
            }
            Self::ShutdownTimeout => formatter.write_str("engine graceful shutdown timed out"),
        }
    }
}

impl std::error::Error for EngineHostError {}

/// Starts and validates the controlled Python analytics process.
#[derive(Debug)]
pub struct EngineHost {
    config: EngineHostConfig,
}

impl EngineHost {
    /// Create an engine host with explicit installation paths and deadlines.
    #[must_use]
    pub const fn new(config: EngineHostConfig) -> Self {
        Self { config }
    }

    /// Spawn the engine, complete its handshake, and return the supervised process.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration is invalid, the process cannot start,
    /// the handshake times out, or identity and health validation fails.
    pub fn start(self) -> Result<RunningEngine, EngineHostError> {
        validate_config(&self.config)?;

        let mut child = Command::new(&self.config.python_executable)
            .args(["-B", "-m", ENGINE_MODULE])
            .current_dir(&self.config.engine_module_root)
            .env("PYTHONPATH", &self.config.engine_module_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(EngineHostError::Spawn)?;

        let Some(mut stdin) = child.stdin.take() else {
            terminate_child(&mut child);
            return Err(EngineHostError::InvalidConfiguration(
                "Python stdin pipe was not created".to_owned(),
            ));
        };
        let Some(stdout) = child.stdout.take() else {
            terminate_child(&mut child);
            return Err(EngineHostError::InvalidConfiguration(
                "Python stdout pipe was not created".to_owned(),
            ));
        };
        let Some(stderr) = child.stderr.take() else {
            terminate_child(&mut child);
            return Err(EngineHostError::InvalidConfiguration(
                "Python stderr pipe was not created".to_owned(),
            ));
        };

        let (receiver, reader_thread) = spawn_stdout_reader(stdout);
        let stderr_capture = Arc::new(Mutex::new(String::new()));
        let stderr_thread = spawn_stderr_reader(stderr, Arc::clone(&stderr_capture));
        let request = EngineHandshakeRequest {
            command: HANDSHAKE_COMMAND.to_owned(),
            host_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: self.config.expected_protocol_version.clone(),
            request_id: self.config.request_id.clone(),
        };

        if let Err(error) = write_message(&mut stdin, &request) {
            terminate_child(&mut child);
            join_thread(reader_thread);
            join_thread(stderr_thread);
            return Err(error);
        }

        let response = match receive_handshake(&receiver, self.config.handshake_timeout)
            .and_then(|line| decode_response(&line))
            .and_then(|response| validate_response(&self.config, response))
        {
            Ok(response) => response,
            Err(error) => {
                drop(stdin);
                terminate_child(&mut child);
                join_thread(reader_thread);
                join_thread(stderr_thread);
                return Err(error);
            }
        };

        Ok(RunningEngine {
            child: Some(child),
            stdin: Some(stdin),
            reader_thread: Some(reader_thread),
            stderr_thread: Some(stderr_thread),
            stderr_capture,
            health: response,
            shutdown_timeout: self.config.shutdown_timeout,
        })
    }
}

/// A handshaken Python process owned exclusively by the native engine host.
#[derive(Debug)]
pub struct RunningEngine {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    reader_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    stderr_capture: Arc<Mutex<String>>,
    health: EngineHandshakeResponse,
    shutdown_timeout: Duration,
}

impl RunningEngine {
    /// Verified startup identity and capabilities.
    #[must_use]
    pub const fn health(&self) -> &EngineHandshakeResponse {
        &self.health
    }

    /// Return true while the child has not exited.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the child process status cannot be queried.
    pub fn is_running(&mut self) -> Result<bool, EngineHostError> {
        let child = self
            .child
            .as_mut()
            .ok_or(EngineHostError::HandshakeChannelClosed)?;
        child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(EngineHostError::Io)
    }

    /// Bounded diagnostic output captured without exposing it to the UI by default.
    #[must_use]
    pub fn stderr_tail(&self) -> String {
        self.stderr_capture
            .lock()
            .map_or_else(|_| String::new(), |capture| capture.clone())
    }

    /// Close stdin and wait for the sidecar's normal EOF shutdown path.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when process status cannot be read, or a timeout error
    /// when forced termination is required.
    pub fn shutdown(&mut self) -> Result<(), EngineHostError> {
        self.stdin.take();
        let Some(child) = self.child.as_mut() else {
            self.join_readers();
            return Ok(());
        };
        let deadline = Instant::now() + self.shutdown_timeout;

        loop {
            if child.try_wait().map_err(EngineHostError::Io)?.is_some() {
                self.child.take();
                self.join_readers();
                return Ok(());
            }
            if Instant::now() >= deadline {
                terminate_child(child);
                self.child.take();
                self.join_readers();
                return Err(EngineHostError::ShutdownTimeout);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn join_readers(&mut self) {
        if let Some(handle) = self.reader_thread.take() {
            join_thread(handle);
        }
        if let Some(handle) = self.stderr_thread.take() {
            join_thread(handle);
        }
    }
}

impl Drop for RunningEngine {
    fn drop(&mut self) {
        if self.shutdown().is_err() {
            if let Some(child) = self.child.as_mut() {
                terminate_child(child);
            }
            self.child.take();
            self.join_readers();
        }
    }
}

fn validate_config(config: &EngineHostConfig) -> Result<(), EngineHostError> {
    if config.request_id.trim().is_empty() {
        return Err(EngineHostError::InvalidConfiguration(
            "request_id cannot be empty".to_owned(),
        ));
    }
    if !config.engine_module_root.is_dir() {
        return Err(EngineHostError::InvalidConfiguration(format!(
            "engine module root does not exist: {}",
            config.engine_module_root.display()
        )));
    }
    if config.handshake_timeout.is_zero() || config.shutdown_timeout.is_zero() {
        return Err(EngineHostError::InvalidConfiguration(
            "timeouts must be greater than zero".to_owned(),
        ));
    }
    Ok(())
}

fn write_message(
    stdin: &mut ChildStdin,
    request: &EngineHandshakeRequest,
) -> Result<(), EngineHostError> {
    serde_json::to_writer(&mut *stdin, request).map_err(EngineHostError::Serialization)?;
    stdin.write_all(b"\n").map_err(EngineHostError::Io)?;
    stdin.flush().map_err(EngineHostError::Io)
}

fn receive_handshake(
    receiver: &Receiver<io::Result<String>>,
    timeout: Duration,
) -> Result<String, EngineHostError> {
    match receiver.recv_timeout(timeout) {
        Ok(Ok(line)) => Ok(line),
        Ok(Err(error)) => Err(EngineHostError::Io(error)),
        Err(RecvTimeoutError::Timeout) => Err(EngineHostError::HandshakeTimeout),
        Err(RecvTimeoutError::Disconnected) => Err(EngineHostError::HandshakeChannelClosed),
    }
}

fn decode_response(line: &str) -> Result<EngineHandshakeResponse, EngineHostError> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(EngineHostError::Serialization)?;
    if value.get("code").is_some() {
        let remote = serde_json::from_value(value).map_err(EngineHostError::Serialization)?;
        return Err(EngineHostError::Remote(Box::new(remote)));
    }
    serde_json::from_value(value).map_err(EngineHostError::Serialization)
}

fn validate_response(
    config: &EngineHostConfig,
    response: EngineHandshakeResponse,
) -> Result<EngineHandshakeResponse, EngineHostError> {
    if response.request_id != config.request_id {
        return Err(EngineHostError::RequestMismatch {
            expected: config.request_id.clone(),
            actual: response.request_id,
        });
    }
    if response.protocol_version != config.expected_protocol_version {
        return Err(EngineHostError::ProtocolMismatch {
            expected: config.expected_protocol_version.clone(),
            actual: response.protocol_version,
        });
    }
    if response.engine_version != config.expected_engine_version {
        return Err(EngineHostError::EngineVersionMismatch {
            expected: config.expected_engine_version.clone(),
            actual: response.engine_version,
        });
    }
    if !response.python_version.starts_with(PYTHON_VERSION_PREFIX) {
        return Err(EngineHostError::PythonVersionMismatch {
            expected: PYTHON_VERSION_PREFIX.to_owned(),
            actual: response.python_version,
        });
    }
    if !response.healthy || response.status != "ready" {
        return Err(EngineHostError::Unhealthy {
            status: response.status,
        });
    }
    if !response
        .capabilities
        .iter()
        .any(|capability| capability == "health")
    {
        return Err(EngineHostError::MissingCapability("health".to_owned()));
    }
    Ok(response)
}

fn spawn_stdout_reader(stdout: ChildStdout) -> (Receiver<io::Result<String>>, JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    (receiver, handle)
}

fn spawn_stderr_reader(stderr: ChildStderr, capture: Arc<Mutex<String>>) -> JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let Ok(mut target) = capture.lock() else {
                break;
            };
            if target.len() >= STDERR_LIMIT_BYTES {
                continue;
            }
            let remaining = STDERR_LIMIT_BYTES - target.len();
            let bounded = line.get(..remaining).unwrap_or(&line);
            target.push_str(bounded);
            target.push('\n');
        }
    })
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn join_thread(handle: JoinHandle<()>) {
    let _ = handle.join();
}

/// Identifies this crate as an initialized workspace component.
#[must_use]
pub const fn component_name() -> &'static str {
    "engine-host"
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;

    use super::*;

    const REQUEST_ID: &str = "00000000-0000-7000-8000-000000000006";

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root")
    }

    fn python_executable() -> PathBuf {
        if let Some(executable) = env::var_os("TERATAI_TEST_PYTHON") {
            return executable.into();
        }
        let workspace = workspace_root();
        let windows_venv = workspace.join(".venv/Scripts/python.exe");
        if windows_venv.is_file() {
            return windows_venv;
        }
        let unix_venv = workspace.join(".venv/bin/python");
        if unix_venv.is_file() {
            return unix_venv;
        }
        PathBuf::from("python")
    }

    fn test_config() -> EngineHostConfig {
        EngineHostConfig::new(
            python_executable(),
            workspace_root().join("engine"),
            REQUEST_ID,
        )
    }

    fn healthy_response() -> EngineHandshakeResponse {
        EngineHandshakeResponse {
            capabilities: vec!["health".to_owned()],
            engine_version: "0.1.0".to_owned(),
            healthy: true,
            protocol_version: "1.0".to_owned(),
            python_version: "3.12.6".to_owned(),
            request_id: REQUEST_ID.to_owned(),
            status: "ready".to_owned(),
        }
    }

    #[test]
    fn exposes_component_name() {
        assert_eq!(component_name(), "engine-host");
    }

    #[test]
    fn starts_handshakes_and_stops_python_sidecar() {
        let mut engine = EngineHost::new(test_config())
            .start()
            .expect("engine handshake must succeed");

        assert_eq!(engine.health().protocol_version, "1.0");
        assert_eq!(engine.health().engine_version, "0.1.0");
        assert!(engine.health().healthy);
        assert!(engine.is_running().expect("engine process state"));
        assert!(engine.stderr_tail().is_empty());
        engine
            .shutdown()
            .expect("engine should stop after stdin closes");
    }

    #[test]
    fn returns_typed_remote_error_for_protocol_mismatch() {
        let mut config = test_config();
        config.expected_protocol_version = "2.0".to_owned();

        let error = EngineHost::new(config)
            .start()
            .expect_err("mismatch must fail");
        match error {
            EngineHostError::Remote(remote) => {
                assert_eq!(remote.code, "ENGINE_UNAVAILABLE");
                assert_eq!(remote.correlation_id, REQUEST_ID);
                assert!(!remote.retriable);
            }
            other => panic!("expected typed remote error, received {other}"),
        }
    }

    #[test]
    fn rejects_unhealthy_or_incompatible_responses() {
        let config = test_config();
        let mut response = healthy_response();
        response.healthy = false;
        response.status = "incompatible-runtime".to_owned();
        assert!(matches!(
            validate_response(&config, response),
            Err(EngineHostError::Unhealthy { .. })
        ));

        let mut response = healthy_response();
        response.python_version = "3.13.0".to_owned();
        assert!(matches!(
            validate_response(&config, response),
            Err(EngineHostError::PythonVersionMismatch { .. })
        ));
    }

    #[test]
    fn handshake_receive_has_a_real_deadline() {
        let (_sender, receiver) = mpsc::channel();
        assert!(matches!(
            receive_handshake(&receiver, Duration::from_millis(1)),
            Err(EngineHostError::HandshakeTimeout)
        ));
    }
}
