mod queue;

pub mod resource;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display, Formatter};
use std::io::Write;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use self::queue::{BoundedQueue, PushError};
use self::resource::{ResourceBudget, ResourceError, ResourceEstimate, ResourceLedger};
use crate::job::{
    JobDescriptor, JobError, JobErrorKind, JobFailureRequest, JobProgressUpdateRequest, JobStatus,
    JobStore, JobTransitionRequest,
};

const RESOURCE_LIMIT_MESSAGE: &str = "Pekerjaan melebihi batas resource yang dikonfigurasi.";
const RESOURCE_INTERNAL_MESSAGE: &str = "Resource executor tidak tersedia untuk pekerjaan ini.";
const PANIC_FAILURE_MESSAGE: &str = "Pekerjaan gagal karena kesalahan operasi internal.";
const HANDLER_PANIC_HOOK_MESSAGE: &str = "Handler job gagal secara internal.";

static HANDLER_PANIC_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

thread_local! {
    static HANDLER_PANIC_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
static HANDLER_PANIC_OBSERVER: Mutex<Option<mpsc::SyncSender<&'static str>>> = Mutex::new(None);

struct HandlerPanicScope {
    previous: bool,
}

impl HandlerPanicScope {
    fn enter() -> Self {
        let previous = HANDLER_PANIC_ACTIVE.with(|active| active.replace(true));
        Self { previous }
    }
}

impl Drop for HandlerPanicScope {
    fn drop(&mut self) {
        HANDLER_PANIC_ACTIVE.with(|active| active.set(self.previous));
    }
}

fn ensure_handler_panic_hook() -> Result<(), JobExecutorError> {
    if thread::panicking() {
        return Err(JobExecutorError::InvalidConfiguration);
    }
    HANDLER_PANIC_HOOK_INSTALLED.get_or_init(|| {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let handler_panic = HANDLER_PANIC_ACTIVE.with(Cell::get);
            if handler_panic {
                emit_sanitized_handler_panic();
            } else {
                previous_hook(panic_info);
            }
        }));
    });
    Ok(())
}

fn emit_sanitized_handler_panic() {
    let mut stderr = std::io::stderr().lock();
    let _message_result = stderr.write_all(HANDLER_PANIC_HOOK_MESSAGE.as_bytes());
    let _newline_result = stderr.write_all(b"\n");
    #[cfg(test)]
    {
        let observer = match HANDLER_PANIC_OBSERVER.lock() {
            Ok(observer) => observer.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Some(observer) = observer {
            let _send_result = observer.try_send(HANDLER_PANIC_HOOK_MESSAGE);
        }
    }
}

#[cfg(test)]
struct HandlerPanicObserverGuard;

#[cfg(test)]
impl Drop for HandlerPanicObserverGuard {
    fn drop(&mut self) {
        let mut observer = match HANDLER_PANIC_OBSERVER.lock() {
            Ok(observer) => observer,
            Err(poisoned) => poisoned.into_inner(),
        };
        *observer = None;
    }
}

#[cfg(test)]
fn set_handler_panic_observer_for_test(
    sender: mpsc::SyncSender<&'static str>,
) -> Result<HandlerPanicObserverGuard, JobExecutorError> {
    let mut observer = HANDLER_PANIC_OBSERVER
        .lock()
        .map_err(|_| JobExecutorError::PersistenceFailed)?;
    if observer.is_some() {
        return Err(JobExecutorError::InvalidConfiguration);
    }
    *observer = Some(sender);
    Ok(HandlerPanicObserverGuard)
}

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
        self.descriptor = self
            .store
            .get(&self.descriptor.job_id)
            .map_err(|error| map_persistence_error(&error))?;
        let status = JobStatus::parse(&self.descriptor.status)
            .map_err(|error| map_persistence_error(&error))?;
        match status {
            JobStatus::Cancelling => return Ok(CheckpointDecision::Cancelled),
            JobStatus::Running => {}
            terminal if terminal.is_terminal() => return Ok(CheckpointDecision::Cancelled),
            _ => return Err(JobExecutorError::InvalidState),
        }
        let timestamp = self
            .clock
            .now()
            .map_err(|_| JobExecutorError::PersistenceFailed)?;
        let request = progress_request(&self.descriptor, &progress);
        match self.store.update_progress_at(&request, &timestamp) {
            Ok(descriptor) => {
                self.descriptor = descriptor;
                Ok(CheckpointDecision::Continue)
            }
            Err(error) if error.kind() == JobErrorKind::RevisionConflict => {
                self.reconcile_progress_conflict(progress)
            }
            Err(error) => Err(map_persistence_error(&error)),
        }
    }

    fn reconcile_progress_conflict(
        &mut self,
        progress: JobProgress,
    ) -> Result<CheckpointDecision, JobExecutorError> {
        let refreshed = self
            .store
            .get(&self.descriptor.job_id)
            .map_err(|error| map_persistence_error(&error))?;
        let status =
            JobStatus::parse(&refreshed.status).map_err(|error| map_persistence_error(&error))?;
        self.descriptor = refreshed;
        match status {
            JobStatus::Cancelling => Ok(CheckpointDecision::Cancelled),
            terminal if terminal.is_terminal() => Ok(CheckpointDecision::Cancelled),
            JobStatus::Running => {
                let timestamp = self
                    .clock
                    .now()
                    .map_err(|_| JobExecutorError::PersistenceFailed)?;
                let request = owned_progress_request(&self.descriptor, progress);
                self.descriptor = self
                    .store
                    .update_progress_at(&request, &timestamp)
                    .map_err(|error| map_persistence_error(&error))?;
                Ok(CheckpointDecision::Continue)
            }
            _ => Err(JobExecutorError::PersistenceConflict),
        }
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

fn progress_request(
    descriptor: &JobDescriptor,
    progress: &JobProgress,
) -> JobProgressUpdateRequest {
    JobProgressUpdateRequest {
        correlation_id: descriptor.correlation_id.clone(),
        current: progress.current,
        expected_revision: descriptor.revision,
        job_id: descriptor.job_id.clone(),
        message: progress.message.clone(),
        phase: progress.phase.clone(),
        total: progress.total,
        unit: progress.unit.clone(),
    }
}

fn owned_progress_request(
    descriptor: &JobDescriptor,
    progress: JobProgress,
) -> JobProgressUpdateRequest {
    JobProgressUpdateRequest {
        correlation_id: descriptor.correlation_id.clone(),
        current: progress.current,
        expected_revision: descriptor.revision,
        job_id: descriptor.job_id.clone(),
        message: progress.message,
        phase: progress.phase,
        total: progress.total,
        unit: progress.unit,
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

struct CompletionReceiver(Mutex<mpsc::Receiver<()>>);

impl CompletionReceiver {
    fn new(receiver: mpsc::Receiver<()>) -> Self {
        Self(Mutex::new(receiver))
    }

    fn recv(&self) -> Result<(), mpsc::RecvError> {
        match self.0.lock() {
            Ok(receiver) => receiver.recv(),
            Err(poisoned) => poisoned.into_inner().recv(),
        }
    }

    fn recv_timeout(&self, timeout: std::time::Duration) -> Result<(), mpsc::RecvTimeoutError> {
        match self.0.lock() {
            Ok(receiver) => receiver.recv_timeout(timeout),
            Err(poisoned) => poisoned.into_inner().recv_timeout(timeout),
        }
    }

    fn try_recv(&self) -> Result<(), mpsc::TryRecvError> {
        match self.0.lock() {
            Ok(receiver) => receiver.try_recv(),
            Err(poisoned) => poisoned.into_inner().try_recv(),
        }
    }
}

struct WorkerHandle {
    join: Option<JoinHandle<()>>,
    completed: CompletionReceiver,
}

enum ReaperCommand {
    AdoptAndStop(Vec<JoinHandle<()>>),
    Stop,
}

struct WorkerReaper {
    commands: mpsc::SyncSender<ReaperCommand>,
    join: Option<JoinHandle<()>>,
    completed: CompletionReceiver,
    stop_requested: bool,
    #[cfg(test)]
    completion_observer: Option<CompletionReceiver>,
}

impl WorkerReaper {
    fn start() -> Result<Self, JobExecutorError> {
        let (commands, receiver) = mpsc::sync_channel(1);
        let (completed_sender, completed) = mpsc::sync_channel(1);
        #[cfg(test)]
        let (observer_sender, completion_observer) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("teratai-job-worker-reaper".to_owned())
            .spawn(move || {
                if let Ok(command) = receiver.recv() {
                    match command {
                        ReaperCommand::AdoptAndStop(workers) => {
                            for worker in workers {
                                let _worker_result = worker.join();
                            }
                        }
                        ReaperCommand::Stop => {}
                    }
                }
                let _completion_result = completed_sender.send(());
                #[cfg(test)]
                let _observer_result = observer_sender.send(());
            })
            .map_err(|_| JobExecutorError::InvalidConfiguration)?;
        Ok(Self {
            commands,
            join: Some(join),
            completed: CompletionReceiver::new(completed),
            stop_requested: false,
            #[cfg(test)]
            completion_observer: Some(CompletionReceiver::new(completion_observer)),
        })
    }

    fn adopt_and_stop(mut self, workers: Vec<JoinHandle<()>>) {
        if self
            .commands
            .send(ReaperCommand::AdoptAndStop(workers))
            .is_ok()
        {
            self.stop_requested = true;
        }
        let _detached_reaper = self.join.take();
    }

    fn stop_and_join_by(&mut self, deadline: Instant) -> bool {
        if self.join.is_none() {
            return true;
        }
        if !self.stop_requested {
            match self.commands.try_send(ReaperCommand::Stop) {
                Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.stop_requested = true;
                }
                Err(mpsc::TrySendError::Full(_)) => return false,
            }
        }

        let completed = wait_for_completion(&self.completed, deadline);
        if completed {
            if let Some(join) = self.join.take() {
                let _reaper_result = join.join();
            }
        }
        completed
    }

    fn stop_and_join(&mut self) {
        if !self.stop_requested {
            let _stop_result = self.commands.send(ReaperCommand::Stop);
            self.stop_requested = true;
        }
        let _completion_result = self.completed.recv();
        if let Some(join) = self.join.take() {
            let _reaper_result = join.join();
        }
    }

    fn detach(mut self) {
        let _detached_reaper = self.join.take();
    }

    #[cfg(test)]
    fn take_completion_for_test(&mut self) -> Option<CompletionReceiver> {
        self.completion_observer.take()
    }
}

/// Project-scoped bounded executor for persisted native jobs.
pub struct JobExecutor {
    core: Arc<ExecutorCore>,
    workers: Vec<WorkerHandle>,
    reaper: Option<WorkerReaper>,
    queue_closed: bool,
    #[cfg(test)]
    shutdown_observer: Option<mpsc::SyncSender<()>>,
}

impl JobExecutor {
    /// Construct the validated core, start its private reaper, and then start workers.
    ///
    /// # Errors
    ///
    /// Returns a safe configuration error when validation, reaper startup, or
    /// worker startup fails. A partial worker set is closed and joined before
    /// construction returns an error.
    pub fn new(
        store: Arc<JobStore>,
        handlers: Vec<Arc<dyn JobHandler>>,
        config: JobExecutorConfig,
        clock: Arc<dyn ExecutorClock>,
    ) -> Result<Self, JobExecutorError> {
        let core = Arc::new(ExecutorCore::new(store, handlers, config, clock)?);
        ensure_handler_panic_hook()?;
        let mut reaper = WorkerReaper::start()?;
        let mut workers = Vec::new();
        workers
            .try_reserve_exact(core.config.worker_count)
            .map_err(|_| {
                reaper.stop_and_join();
                JobExecutorError::InvalidConfiguration
            })?;

        for worker_index in 0..core.config.worker_count {
            let worker_core = Arc::clone(&core);
            let (completed_sender, completed) = mpsc::sync_channel(1);
            let spawn_result = thread::Builder::new()
                .name(format!("teratai-job-worker-{worker_index}"))
                .spawn(move || {
                    worker_loop(&worker_core);
                    let _completion_result = completed_sender.send(());
                });
            if let Ok(join) = spawn_result {
                workers.push(WorkerHandle {
                    join: Some(join),
                    completed: CompletionReceiver::new(completed),
                });
            } else {
                rollback_worker_start(&core, &mut workers, &mut reaper);
                return Err(JobExecutorError::InvalidConfiguration);
            }
        }

        Ok(Self {
            core,
            workers,
            reaper: Some(reaper),
            queue_closed: false,
            #[cfg(test)]
            shutdown_observer: None,
        })
    }

    /// Submit one already-persisted queued job without waiting for capacity.
    ///
    /// # Errors
    ///
    /// Returns a safe admission error without mutating persistent state.
    pub fn submit(&self, job_id: &str) -> Result<(), JobExecutorError> {
        self.core.claim_and_queue(job_id)
    }

    /// Stop accepting work, drain pending IDs, and join all executor threads
    /// within the configured deadline.
    ///
    /// A timed-out worker remains owned by the executor so callers can release
    /// cooperative work and retry shutdown without losing its join handle.
    ///
    /// # Errors
    ///
    /// Returns [`JobExecutorError::ShutdownTimeout`] when any worker or the
    /// private reaper cannot be joined before the single shutdown deadline.
    pub fn shutdown(&mut self) -> Result<(), JobExecutorError> {
        self.close_queue_once();
        let deadline = Instant::now()
            .checked_add(self.core.config.shutdown_timeout)
            .ok_or(JobExecutorError::InvalidConfiguration)?;

        for worker in &mut self.workers {
            if worker.join.is_none() {
                continue;
            }
            if wait_for_completion(&worker.completed, deadline) {
                if let Some(join) = worker.join.take() {
                    let _worker_result = join.join();
                }
            }
        }
        if self.workers.iter().any(|worker| worker.join.is_some()) {
            return Err(JobExecutorError::ShutdownTimeout);
        }

        if let Some(reaper) = &mut self.reaper {
            if !reaper.stop_and_join_by(deadline) {
                return Err(JobExecutorError::ShutdownTimeout);
            }
        }
        self.reaper = None;
        Ok(())
    }

    fn close_queue_once(&mut self) {
        if self.queue_closed {
            return;
        }
        let drained = self.core.queue.close_and_drain();
        release_drained_admissions(&self.core.admitted, drained);
        self.queue_closed = true;
        #[cfg(test)]
        if let Some(observer) = &self.shutdown_observer {
            let _observer_result = observer.try_send(());
        }
    }

    #[cfg(test)]
    fn set_shutdown_observer_for_test(
        &mut self,
        observer: mpsc::SyncSender<()>,
    ) -> Result<(), JobExecutorError> {
        if self.shutdown_observer.is_some() || self.queue_closed {
            return Err(JobExecutorError::InvalidConfiguration);
        }
        self.shutdown_observer = Some(observer);
        Ok(())
    }

    #[cfg(test)]
    fn take_reaper_completion_for_test(&mut self) -> Option<CompletionReceiver> {
        self.reaper
            .as_mut()
            .and_then(WorkerReaper::take_completion_for_test)
    }
}

impl Drop for JobExecutor {
    fn drop(&mut self) {
        let _shutdown_result = self.shutdown();
        let outstanding: Vec<_> = self
            .workers
            .iter_mut()
            .filter_map(|worker| worker.join.take())
            .collect();
        if let Some(reaper) = self.reaper.take() {
            if outstanding.is_empty() {
                reaper.detach();
            } else {
                reaper.adopt_and_stop(outstanding);
            }
        }
    }
}

fn wait_for_completion(completed: &CompletionReceiver, deadline: Instant) -> bool {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return matches!(
            completed.try_recv(),
            Ok(()) | Err(mpsc::TryRecvError::Disconnected)
        );
    }
    matches!(
        completed.recv_timeout(remaining),
        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected)
    )
}

fn rollback_worker_start(
    core: &ExecutorCore,
    workers: &mut [WorkerHandle],
    reaper: &mut WorkerReaper,
) {
    let drained = core.queue.close_and_drain();
    release_drained_admissions(&core.admitted, drained);
    for worker in workers {
        if let Some(join) = worker.join.take() {
            let _worker_result = join.join();
        }
    }
    reaper.stop_and_join();
}

fn release_drained_admissions(admitted: &Mutex<HashSet<String>>, drained: Vec<String>) {
    let mut admitted = match admitted.lock() {
        Ok(admitted) => admitted,
        Err(poisoned) => poisoned.into_inner(),
    };
    for job_id in drained {
        admitted.remove(&job_id);
    }
}

struct AdmissionGuard {
    admitted: Arc<Mutex<HashSet<String>>>,
    job_id: String,
}

impl AdmissionGuard {
    fn new(admitted: Arc<Mutex<HashSet<String>>>, job_id: String) -> Self {
        Self { admitted, job_id }
    }
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        let mut admitted = match self.admitted.lock() {
            Ok(admitted) => admitted,
            Err(poisoned) => poisoned.into_inner(),
        };
        admitted.remove(&self.job_id);
    }
}

fn worker_loop(core: &ExecutorCore) {
    while let Some(job_id) = core.queue.pop() {
        {
            let _admission = AdmissionGuard::new(Arc::clone(&core.admitted), job_id.clone());
            let _execution_result = execute_job(core, &job_id);
        }
        #[cfg(test)]
        core.run_after_job_hook(&job_id);
    }
}

fn execute_job(core: &ExecutorCore, job_id: &str) -> Result<(), JobExecutorError> {
    let queued = core
        .store
        .get(job_id)
        .map_err(|error| map_persistence_error(&error))?;
    match JobStatus::parse(&queued.status).map_err(|error| map_persistence_error(&error))? {
        JobStatus::Queued => {}
        JobStatus::Cancelling => return complete_cancellation_from_snapshot(core, &queued),
        _ => return Ok(()),
    }
    let handler = core
        .handlers
        .get(queued.kind.as_str())
        .cloned()
        .ok_or(JobExecutorError::HandlerNotFound)?;
    let reservation = match core.resource_ledger.reserve(handler.estimate()) {
        Ok(reservation) => reservation,
        Err(
            ResourceError::MemoryExceeded
            | ResourceError::DiskExceeded
            | ResourceError::DurationExceeded
            | ResourceError::ArithmeticOverflow,
        ) => {
            return persist_terminal_failure(
                core,
                job_id,
                TerminalFailure {
                    code: "RESOURCE_LIMIT",
                    message: RESOURCE_LIMIT_MESSAGE,
                    retriable: false,
                },
                FailureOrigin::Preflight,
            );
        }
        Err(ResourceError::InvalidBudget | ResourceError::Poisoned) => {
            return persist_terminal_failure(
                core,
                job_id,
                TerminalFailure {
                    code: "OPERATION_FAILED",
                    message: RESOURCE_INTERNAL_MESSAGE,
                    retriable: false,
                },
                FailureOrigin::Preflight,
            );
        }
    };
    let _reservation = reservation;
    let started_at = core
        .clock
        .now()
        .map_err(|_| JobExecutorError::PersistenceFailed)?;
    let running = match core
        .store
        .start_at(&transition_request(&queued), &started_at)
    {
        Ok(running) => running,
        Err(error) if error.kind() == JobErrorKind::RevisionConflict => {
            return reconcile_start_conflict(core, job_id);
        }
        Err(error) => return Err(map_persistence_error(&error)),
    };
    let mut context = ExecutionContext {
        store: core.store.as_ref(),
        descriptor: running,
        clock: core.clock.as_ref(),
    };

    let handler_result = {
        let _panic_scope = HandlerPanicScope::enter();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler.run(&mut context)))
    };
    match handler_result {
        Ok(Ok(HandlerOutcome::Completed)) => finish_completed_job(core, job_id)?,
        Ok(Ok(HandlerOutcome::Cancelled)) => finish_cancelled_job(core, job_id)?,
        Ok(Err(error)) => persist_terminal_failure(
            core,
            job_id,
            TerminalFailure {
                code: error.code(),
                message: error.message(),
                retriable: error.retriable(),
            },
            FailureOrigin::Handler,
        )?,
        Err(_) => persist_terminal_failure(
            core,
            job_id,
            TerminalFailure {
                code: "OPERATION_FAILED",
                message: PANIC_FAILURE_MESSAGE,
                retriable: false,
            },
            FailureOrigin::Handler,
        )?,
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct TerminalFailure<'a> {
    code: &'a str,
    message: &'a str,
    retriable: bool,
}

#[derive(Clone, Copy)]
enum FailureOrigin {
    Preflight,
    Handler,
}

impl FailureOrigin {
    const fn accepts(self, status: JobStatus) -> bool {
        matches!(
            (self, status),
            (Self::Preflight, JobStatus::Queued) | (Self::Handler, JobStatus::Running)
        )
    }
}

fn persist_terminal_failure(
    core: &ExecutorCore,
    job_id: &str,
    failure: TerminalFailure<'_>,
    origin: FailureOrigin,
) -> Result<(), JobExecutorError> {
    let descriptor = core
        .store
        .get(job_id)
        .map_err(|error| map_persistence_error(&error))?;
    fail_from_snapshot(core, &descriptor, failure, origin, true)
}

fn fail_from_snapshot(
    core: &ExecutorCore,
    descriptor: &JobDescriptor,
    failure: TerminalFailure<'_>,
    origin: FailureOrigin,
    reconcile_once: bool,
) -> Result<(), JobExecutorError> {
    let status =
        JobStatus::parse(&descriptor.status).map_err(|error| map_persistence_error(&error))?;
    if status == JobStatus::Cancelling {
        return complete_cancellation_from_snapshot(core, descriptor);
    }
    if status.is_terminal() {
        return Ok(());
    }
    if !origin.accepts(status) {
        return Err(JobExecutorError::PersistenceConflict);
    }

    let failed_at = core
        .clock
        .now()
        .map_err(|_| JobExecutorError::PersistenceFailed)?;
    let request = JobFailureRequest {
        correlation_id: descriptor.correlation_id.clone(),
        error_code: failure.code.to_owned(),
        error_message: failure.message.to_owned(),
        error_retriable: failure.retriable,
        expected_revision: descriptor.revision,
        job_id: descriptor.job_id.clone(),
    };
    match core.store.fail_at(&request, &failed_at) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == JobErrorKind::RevisionConflict && reconcile_once => {
            let refreshed = core
                .store
                .get(&descriptor.job_id)
                .map_err(|read_error| map_persistence_error(&read_error))?;
            fail_from_snapshot(core, &refreshed, failure, origin, false)
        }
        Err(error) => Err(map_persistence_error(&error)),
    }
}

fn reconcile_start_conflict(core: &ExecutorCore, job_id: &str) -> Result<(), JobExecutorError> {
    let refreshed = core
        .store
        .get(job_id)
        .map_err(|error| map_persistence_error(&error))?;
    let status =
        JobStatus::parse(&refreshed.status).map_err(|error| map_persistence_error(&error))?;
    match status {
        JobStatus::Cancelling => complete_cancellation_from_snapshot(core, &refreshed),
        terminal if terminal.is_terminal() => Ok(()),
        _ => Err(JobExecutorError::PersistenceConflict),
    }
}

fn finish_completed_job(core: &ExecutorCore, job_id: &str) -> Result<(), JobExecutorError> {
    let descriptor = core
        .store
        .get(job_id)
        .map_err(|error| map_persistence_error(&error))?;
    let status =
        JobStatus::parse(&descriptor.status).map_err(|error| map_persistence_error(&error))?;
    match status {
        JobStatus::Running => {
            let finished_at = core
                .clock
                .now()
                .map_err(|_| JobExecutorError::PersistenceFailed)?;
            let request = transition_request(&descriptor);
            match core.store.succeed_at(&request, &finished_at) {
                Ok(_) => Ok(()),
                Err(error) if error.kind() == JobErrorKind::RevisionConflict => {
                    reconcile_completion_conflict(core, job_id)
                }
                Err(error) => Err(map_persistence_error(&error)),
            }
        }
        JobStatus::Cancelling => complete_cancellation_from_snapshot(core, &descriptor),
        terminal if terminal.is_terminal() => Ok(()),
        _ => Err(JobExecutorError::PersistenceConflict),
    }
}

fn finish_cancelled_job(core: &ExecutorCore, job_id: &str) -> Result<(), JobExecutorError> {
    let descriptor = core
        .store
        .get(job_id)
        .map_err(|error| map_persistence_error(&error))?;
    let status =
        JobStatus::parse(&descriptor.status).map_err(|error| map_persistence_error(&error))?;
    match status {
        JobStatus::Cancelling => complete_cancellation_from_snapshot(core, &descriptor),
        terminal if terminal.is_terminal() => Ok(()),
        _ => Err(JobExecutorError::InvalidState),
    }
}

fn reconcile_completion_conflict(
    core: &ExecutorCore,
    job_id: &str,
) -> Result<(), JobExecutorError> {
    let refreshed = core
        .store
        .get(job_id)
        .map_err(|error| map_persistence_error(&error))?;
    let status =
        JobStatus::parse(&refreshed.status).map_err(|error| map_persistence_error(&error))?;
    match status {
        JobStatus::Cancelling => complete_cancellation_from_snapshot(core, &refreshed),
        terminal if terminal.is_terminal() => Ok(()),
        _ => Err(JobExecutorError::PersistenceConflict),
    }
}

fn complete_cancellation_from_snapshot(
    core: &ExecutorCore,
    descriptor: &JobDescriptor,
) -> Result<(), JobExecutorError> {
    let finished_at = core
        .clock
        .now()
        .map_err(|_| JobExecutorError::PersistenceFailed)?;
    match core
        .store
        .complete_cancellation_at(&transition_request(descriptor), &finished_at)
    {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == JobErrorKind::RevisionConflict => {
            reconcile_cancellation_completion(core, &descriptor.job_id)
        }
        Err(error) => Err(map_persistence_error(&error)),
    }
}

fn reconcile_cancellation_completion(
    core: &ExecutorCore,
    job_id: &str,
) -> Result<(), JobExecutorError> {
    let refreshed = core
        .store
        .get(job_id)
        .map_err(|error| map_persistence_error(&error))?;
    let status =
        JobStatus::parse(&refreshed.status).map_err(|error| map_persistence_error(&error))?;
    match status {
        JobStatus::Cancelling => {
            let finished_at = core
                .clock
                .now()
                .map_err(|_| JobExecutorError::PersistenceFailed)?;
            core.store
                .complete_cancellation_at(&transition_request(&refreshed), &finished_at)
                .map(|_| ())
                .map_err(|error| map_persistence_error(&error))
        }
        terminal if terminal.is_terminal() => Ok(()),
        _ => Err(JobExecutorError::PersistenceConflict),
    }
}

fn transition_request(descriptor: &JobDescriptor) -> JobTransitionRequest {
    JobTransitionRequest {
        correlation_id: descriptor.correlation_id.clone(),
        expected_revision: descriptor.revision,
        job_id: descriptor.job_id.clone(),
    }
}

pub(crate) struct ExecutorCore {
    store: Arc<JobStore>,
    queue: Arc<BoundedQueue<String>>,
    handlers: HashMap<&'static str, Arc<dyn JobHandler>>,
    admitted: Arc<Mutex<HashSet<String>>>,
    resource_ledger: ResourceLedger,
    config: JobExecutorConfig,
    clock: Arc<dyn ExecutorClock>,
    #[cfg(test)]
    admission_bound: usize,
    #[cfg(test)]
    before_queue_push: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    after_job: Mutex<Option<AfterJobHook>>,
}

#[cfg(test)]
type AfterJobHook = Arc<dyn Fn(&str) + Send + Sync>;

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
            resource_ledger,
            config,
            clock,
            #[cfg(test)]
            admission_bound,
            #[cfg(test)]
            before_queue_push: Mutex::new(None),
            #[cfg(test)]
            after_job: Mutex::new(None),
        })
    }

    pub(crate) fn claim_and_queue(&self, job_id: &str) -> Result<(), JobExecutorError> {
        {
            let admitted = self
                .admitted
                .lock()
                .map_err(|_| JobExecutorError::PersistenceFailed)?;
            if admitted.contains(job_id) {
                return Err(JobExecutorError::AlreadySubmitted);
            }
        }
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

    #[cfg(test)]
    fn set_after_job_hook(&self, hook: AfterJobHook) -> Result<(), JobExecutorError> {
        *self
            .after_job
            .lock()
            .map_err(|_| JobExecutorError::PersistenceFailed)? = Some(hook);
        Ok(())
    }

    #[cfg(test)]
    fn run_after_job_hook(&self, job_id: &str) {
        let hook = match self.after_job.lock() {
            Ok(hook) => hook.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Some(hook) = hook {
            hook(job_id);
        }
    }
}

fn validate_config(config: &JobExecutorConfig) -> Result<(), JobExecutorError> {
    let available = std::thread::available_parallelism()
        .map_err(|_| JobExecutorError::InvalidConfiguration)?
        .get();
    if config.worker_count == 0
        || config.worker_count > available
        || config.queue_capacity == 0
        || config.shutdown_timeout.is_zero()
        || Instant::now()
            .checked_add(config.shutdown_timeout)
            .is_none()
    {
        return Err(JobExecutorError::InvalidConfiguration);
    }
    Ok(())
}

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
