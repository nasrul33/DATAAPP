mod queue;

pub mod resource;

use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display, Formatter};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use self::queue::{BoundedQueue, PushError};
use self::resource::{ResourceBudget, ResourceEstimate, ResourceLedger};
use crate::job::{
    JobDescriptor, JobError, JobErrorKind, JobProgressUpdateRequest, JobStatus, JobStore,
    JobTransitionRequest,
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

struct WorkerHandle {
    join: Option<JoinHandle<()>>,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Task 7 consumes the required completion signal for deadline-based shutdown"
        )
    )]
    completed: mpsc::Receiver<()>,
}

enum ReaperCommand {
    Adopt(JoinHandle<()>),
    Stop,
}

struct WorkerReaper {
    commands: mpsc::SyncSender<ReaperCommand>,
    join: Option<JoinHandle<()>>,
}

impl WorkerReaper {
    fn start(capacity: usize) -> Result<Self, JobExecutorError> {
        let (commands, receiver) = mpsc::sync_channel(capacity);
        let join = thread::Builder::new()
            .name("teratai-job-worker-reaper".to_owned())
            .spawn(move || {
                while let Ok(command) = receiver.recv() {
                    match command {
                        ReaperCommand::Adopt(worker) => {
                            let _worker_result = worker.join();
                        }
                        ReaperCommand::Stop => break,
                    }
                }
            })
            .map_err(|_| JobExecutorError::InvalidConfiguration)?;
        Ok(Self {
            commands,
            join: Some(join),
        })
    }

    fn adopt(&self, worker: JoinHandle<()>) {
        if let Err(error) = self.commands.send(ReaperCommand::Adopt(worker)) {
            if let ReaperCommand::Adopt(worker) = error.0 {
                let _worker_result = worker.join();
            }
        }
    }

    fn request_stop(&self) {
        let _stop_result = self.commands.send(ReaperCommand::Stop);
    }

    fn stop_and_join(&mut self) {
        self.request_stop();
        if let Some(join) = self.join.take() {
            let _reaper_result = join.join();
        }
    }
}

/// Project-scoped bounded executor for persisted native jobs.
pub struct JobExecutor {
    core: Arc<ExecutorCore>,
    workers: Vec<WorkerHandle>,
    reaper: Option<WorkerReaper>,
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
        let reaper_capacity = core
            .config
            .worker_count
            .checked_add(1)
            .ok_or(JobExecutorError::InvalidConfiguration)?;
        let mut reaper = WorkerReaper::start(reaper_capacity)?;
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
                    completed,
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
}

impl Drop for JobExecutor {
    fn drop(&mut self) {
        let drained = self.core.queue.close_and_drain();
        release_drained_admissions(&self.core.admitted, drained);
        if let Some(reaper) = self.reaper.take() {
            for worker in &mut self.workers {
                if let Some(join) = worker.join.take() {
                    reaper.adopt(join);
                }
            }
            reaper.request_stop();
        }
    }
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
        let _admission = AdmissionGuard::new(Arc::clone(&core.admitted), job_id.clone());
        let _execution_result = execute_job(core, &job_id);
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
    let _reservation = core
        .resource_ledger
        .reserve(handler.estimate())
        .map_err(|_| JobExecutorError::PreflightRejected)?;
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

    match handler.run(&mut context) {
        Ok(HandlerOutcome::Completed) => finish_completed_job(core, job_id)?,
        Ok(HandlerOutcome::Cancelled) => finish_cancelled_job(core, job_id)?,
        Err(_) => {}
    }
    Ok(())
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
    let finished_at = core
        .clock
        .now()
        .map_err(|_| JobExecutorError::PersistenceFailed)?;
    let request = transition_request(&descriptor);
    match status {
        JobStatus::Running => match core.store.succeed_at(&request, &finished_at) {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == JobErrorKind::RevisionConflict => {
                reconcile_completion_conflict(core, job_id)
            }
            Err(error) => Err(map_persistence_error(&error)),
        },
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
}

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
