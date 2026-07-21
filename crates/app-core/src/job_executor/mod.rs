#[expect(
    dead_code,
    reason = "worker-only queue paths remain intentionally unused until Task 4 starts executor threads"
)]
mod queue;

pub mod resource;

use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display, Formatter};
use std::sync::{Arc, Mutex};

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use self::queue::{BoundedQueue, PushError};
use self::resource::{ResourceBudget, ResourceEstimate, ResourceLedger};
use crate::job::{
    JobDescriptor, JobError, JobErrorKind, JobProgressUpdateRequest, JobStatus, JobStore,
};

#[derive(Debug, Clone)]
pub struct JobExecutorConfig {
    pub worker_count: usize,
    pub queue_capacity: usize,
    pub resource_budget: ResourceBudget,
    pub shutdown_timeout: std::time::Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockError;

pub trait ExecutorClock: Send + Sync {
    /// Return one UTC RFC 3339 timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError`] when a timestamp cannot be produced or encoded.
    fn now(&self) -> Result<String, ClockError>;
}

#[derive(Debug, Default)]
pub struct SystemExecutorClock;

impl ExecutorClock for SystemExecutorClock {
    fn now(&self) -> Result<String, ClockError> {
        OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|_| ClockError)
    }
}

pub trait JobHandler: Send + Sync {
    fn kind(&self) -> &'static str;
    fn estimate(&self) -> ResourceEstimate;

    /// Run deterministic native work through the executor-owned context.
    ///
    /// # Errors
    ///
    /// Returns only a validated, source-free handler failure.
    fn run(&self, context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerOutcome {
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobProgress {
    pub current: i64,
    pub total: Option<i64>,
    pub unit: Option<String>,
    pub phase: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointDecision {
    Continue,
    Cancelled,
}

pub struct ExecutionContext<'a> {
    store: &'a JobStore,
    descriptor: JobDescriptor,
    clock: &'a dyn ExecutorClock,
}

impl ExecutionContext<'_> {
    /// Persist one validated progress checkpoint unless cancellation is pending.
    ///
    /// # Errors
    ///
    /// Returns a safe executor error when the clock or persistent store cannot
    /// complete the checkpoint.
    pub fn checkpoint(
        &mut self,
        progress: JobProgress,
    ) -> Result<CheckpointDecision, JobExecutorError> {
        if self.cancellation_requested()? {
            return Ok(CheckpointDecision::Cancelled);
        }
        let timestamp = self
            .clock
            .now()
            .map_err(|_| JobExecutorError::PersistenceFailed)?;
        let request = JobProgressUpdateRequest {
            correlation_id: self.descriptor.correlation_id.clone(),
            current: progress.current,
            expected_revision: self.descriptor.revision,
            job_id: self.descriptor.job_id.clone(),
            message: progress.message,
            phase: progress.phase,
            total: progress.total,
            unit: progress.unit,
        };
        self.descriptor = self
            .store
            .update_progress_at(&request, &timestamp)
            .map_err(|error| map_persistence_error(&error))?;
        Ok(CheckpointDecision::Continue)
    }

    /// Refresh the trusted snapshot and report cooperative cancellation state.
    ///
    /// # Errors
    ///
    /// Returns a safe executor error when the snapshot cannot be read or is
    /// internally inconsistent.
    pub fn cancellation_requested(&mut self) -> Result<bool, JobExecutorError> {
        self.descriptor = self
            .store
            .get(&self.descriptor.job_id)
            .map_err(|error| map_persistence_error(&error))?;
        JobStatus::parse(&self.descriptor.status)
            .map(|status| status == JobStatus::Cancelling)
            .map_err(|error| map_persistence_error(&error))
    }

    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.descriptor.job_id
    }

    #[must_use]
    pub fn project_id(&self) -> &str {
        &self.descriptor.project_id
    }

    #[must_use]
    pub fn correlation_id(&self) -> &str {
        &self.descriptor.correlation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobHandlerError {
    code: String,
    message: String,
    retriable: bool,
}

impl JobHandlerError {
    /// Construct a source-free, bounded handler failure.
    ///
    /// # Errors
    ///
    /// Returns [`JobExecutorError::InvalidConfiguration`] when either safe
    /// field violates its stable contract.
    pub fn new(code: &str, message: &str, retriable: bool) -> Result<Self, JobExecutorError> {
        if !is_safe_error_code(code) || !is_safe_message(message, 500) {
            return Err(JobExecutorError::InvalidConfiguration);
        }
        Ok(Self {
            code: code.to_owned(),
            message: message.to_owned(),
            retriable,
        })
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn retriable(&self) -> bool {
        self.retriable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobExecutorErrorKind {
    InvalidConfiguration,
    InvalidJobId,
    JobNotFound,
    InvalidState,
    HandlerNotFound,
    AlreadySubmitted,
    QueueFull,
    PreflightRejected,
    PersistenceConflict,
    PersistenceFailed,
    ShuttingDown,
    ShutdownTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobExecutorError {
    InvalidConfiguration,
    InvalidJobId,
    JobNotFound,
    InvalidState,
    HandlerNotFound,
    AlreadySubmitted,
    QueueFull,
    PreflightRejected,
    PersistenceConflict,
    PersistenceFailed,
    ShuttingDown,
    ShutdownTimeout,
}

impl JobExecutorError {
    #[must_use]
    pub const fn kind(&self) -> JobExecutorErrorKind {
        match self {
            Self::InvalidConfiguration => JobExecutorErrorKind::InvalidConfiguration,
            Self::InvalidJobId => JobExecutorErrorKind::InvalidJobId,
            Self::JobNotFound => JobExecutorErrorKind::JobNotFound,
            Self::InvalidState => JobExecutorErrorKind::InvalidState,
            Self::HandlerNotFound => JobExecutorErrorKind::HandlerNotFound,
            Self::AlreadySubmitted => JobExecutorErrorKind::AlreadySubmitted,
            Self::QueueFull => JobExecutorErrorKind::QueueFull,
            Self::PreflightRejected => JobExecutorErrorKind::PreflightRejected,
            Self::PersistenceConflict => JobExecutorErrorKind::PersistenceConflict,
            Self::PersistenceFailed => JobExecutorErrorKind::PersistenceFailed,
            Self::ShuttingDown => JobExecutorErrorKind::ShuttingDown,
            Self::ShutdownTimeout => JobExecutorErrorKind::ShutdownTimeout,
        }
    }
}

impl Display for JobExecutorError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfiguration => "konfigurasi executor tidak valid",
            Self::InvalidJobId => "identitas job tidak valid",
            Self::JobNotFound => "job tidak ditemukan",
            Self::InvalidState => "status job tidak dapat dijalankan",
            Self::HandlerNotFound => "handler job tidak tersedia",
            Self::AlreadySubmitted => "job sudah diajukan",
            Self::QueueFull => "antrean job penuh",
            Self::PreflightRejected => "batas resource menolak job",
            Self::PersistenceConflict => "revisi job berubah",
            Self::PersistenceFailed => "penyimpanan job gagal",
            Self::ShuttingDown => "executor sedang berhenti",
            Self::ShutdownTimeout => "batas waktu penghentian executor terlampaui",
        })
    }
}

impl std::error::Error for JobExecutorError {}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Task 3 validates the crate-private core; Task 4 supplies its first production owner"
    )
)]
pub(crate) struct ExecutorCore {
    store: Arc<JobStore>,
    queue: Arc<BoundedQueue<String>>,
    handlers: HashMap<&'static str, Arc<dyn JobHandler>>,
    admitted: Arc<Mutex<HashSet<String>>>,
    _resource_ledger: ResourceLedger,
    _config: JobExecutorConfig,
    _clock: Arc<dyn ExecutorClock>,
    #[cfg(test)]
    admission_bound: usize,
    #[cfg(test)]
    before_queue_push: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Task 3 validates admission in unit tests; Task 4 calls these methods from the public executor"
    )
)]
impl ExecutorCore {
    pub(crate) fn new(
        store: Arc<JobStore>,
        handlers: Vec<Arc<dyn JobHandler>>,
        config: JobExecutorConfig,
        clock: Arc<dyn ExecutorClock>,
    ) -> Result<Self, JobExecutorError> {
        validate_config(&config)?;
        let admission_bound = config
            .queue_capacity
            .checked_add(config.worker_count)
            .and_then(|bound| bound.checked_add(1))
            .ok_or(JobExecutorError::InvalidConfiguration)?;
        let queue = Arc::new(
            BoundedQueue::new(config.queue_capacity)
                .ok_or(JobExecutorError::InvalidConfiguration)?,
        );
        let resource_ledger = ResourceLedger::new(config.resource_budget)
            .map_err(|_| JobExecutorError::InvalidConfiguration)?;

        let mut registry = HashMap::new();
        registry
            .try_reserve(handlers.len())
            .map_err(|_| JobExecutorError::InvalidConfiguration)?;
        for handler in handlers {
            let kind = handler.kind();
            if !is_safe_token(kind, 120) || registry.insert(kind, handler).is_some() {
                return Err(JobExecutorError::InvalidConfiguration);
            }
        }

        let mut admitted = HashSet::new();
        admitted
            .try_reserve(admission_bound)
            .map_err(|_| JobExecutorError::InvalidConfiguration)?;
        Ok(Self {
            store,
            queue,
            handlers: registry,
            admitted: Arc::new(Mutex::new(admitted)),
            _resource_ledger: resource_ledger,
            _config: config,
            _clock: clock,
            #[cfg(test)]
            admission_bound,
            #[cfg(test)]
            before_queue_push: Mutex::new(None),
        })
    }

    pub(crate) fn claim_and_queue(&self, job_id: &str) -> Result<(), JobExecutorError> {
        let descriptor = self
            .store
            .get(job_id)
            .map_err(|error| map_admission_read_error(&error))?;
        let status =
            JobStatus::parse(&descriptor.status).map_err(|error| map_persistence_error(&error))?;
        if status != JobStatus::Queued {
            return Err(JobExecutorError::InvalidState);
        }
        if !self.handlers.contains_key(descriptor.kind.as_str()) {
            return Err(JobExecutorError::HandlerNotFound);
        }

        let mut admitted = self
            .admitted
            .lock()
            .map_err(|_| JobExecutorError::PersistenceFailed)?;
        if !admitted.insert(descriptor.job_id.clone()) {
            return Err(JobExecutorError::AlreadySubmitted);
        }

        #[cfg(test)]
        if let Err(error) = self.run_before_queue_push_hook() {
            admitted.remove(&descriptor.job_id);
            return Err(error);
        }

        match self.queue.try_push(descriptor.job_id) {
            Ok(()) => Ok(()),
            Err(error) => {
                let (job_id, mapped) = match error {
                    PushError::Full(job_id) => (job_id, JobExecutorError::QueueFull),
                    PushError::Closed(job_id) => (job_id, JobExecutorError::ShuttingDown),
                    PushError::Poisoned(job_id) => (job_id, JobExecutorError::PersistenceFailed),
                };
                admitted.remove(&job_id);
                Err(mapped)
            }
        }
    }

    #[cfg(test)]
    fn set_before_queue_push_hook(
        &self,
        hook: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<(), JobExecutorError> {
        *self
            .before_queue_push
            .lock()
            .map_err(|_| JobExecutorError::PersistenceFailed)? = Some(hook);
        Ok(())
    }

    #[cfg(test)]
    fn run_before_queue_push_hook(&self) -> Result<(), JobExecutorError> {
        let hook = self
            .before_queue_push
            .lock()
            .map_err(|_| JobExecutorError::PersistenceFailed)?
            .clone();
        if let Some(hook) = hook {
            hook();
        }
        Ok(())
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Task 3 validates construction in unit tests; Task 4 calls it through ExecutorCore"
    )
)]
fn validate_config(config: &JobExecutorConfig) -> Result<(), JobExecutorError> {
    let available = std::thread::available_parallelism()
        .map_err(|_| JobExecutorError::InvalidConfiguration)?
        .get();
    if config.worker_count == 0
        || config.worker_count > available
        || config.queue_capacity == 0
        || config.shutdown_timeout.is_zero()
    {
        return Err(JobExecutorError::InvalidConfiguration);
    }
    Ok(())
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Task 3 validates admission in unit tests; Task 4 makes the core production-live"
    )
)]
fn map_admission_read_error(error: &JobError) -> JobExecutorError {
    match error.kind() {
        JobErrorKind::InvalidRequest => JobExecutorError::InvalidJobId,
        JobErrorKind::JobNotFound => JobExecutorError::JobNotFound,
        _ => JobExecutorError::PersistenceFailed,
    }
}

fn map_persistence_error(error: &JobError) -> JobExecutorError {
    match error.kind() {
        JobErrorKind::JobNotFound => JobExecutorError::JobNotFound,
        JobErrorKind::RevisionConflict => JobExecutorError::PersistenceConflict,
        _ => JobExecutorError::PersistenceFailed,
    }
}

fn is_safe_error_code(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=120).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
}

fn is_safe_message(value: &str, maximum: usize) -> bool {
    (1..=maximum).contains(&value.len())
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Task 3 validates handler registration in unit tests; Task 4 makes the registry production-live"
    )
)]
fn is_safe_token(value: &str, maximum: usize) -> bool {
    let bytes = value.as_bytes();
    (1..=maximum).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..].iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

#[cfg(test)]
mod tests;
