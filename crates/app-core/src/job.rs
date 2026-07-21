use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior,
};
use sha2::{Digest, Sha256};
pub use teratai_contracts::generated::job_descriptor::JobDescriptor;
pub use teratai_contracts::generated::job_enqueue_request::JobEnqueueRequest;
pub use teratai_contracts::generated::job_failure_request::JobFailureRequest;
pub use teratai_contracts::generated::job_progress_update_request::JobProgressUpdateRequest;
pub use teratai_contracts::generated::job_transition_request::JobTransitionRequest;
use teratai_filesystem::{
    pin_project_metadata, validate_project_layout, PinnedProjectMetadata, PinnedProjectOperation,
    ProjectLayout,
};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

#[cfg(test)]
type AfterCommitBarrier = Option<(std::thread::ThreadId, std::sync::Arc<std::sync::Barrier>)>;

#[cfg(test)]
static AFTER_COMMIT_BARRIER: std::sync::OnceLock<std::sync::Mutex<AfterCommitBarrier>> =
    std::sync::OnceLock::new();

#[cfg(test)]
fn install_after_commit_barrier(barrier: Option<std::sync::Arc<std::sync::Barrier>>) {
    *AFTER_COMMIT_BARRIER
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("after-commit barrier is available") =
        barrier.map(|value| (std::thread::current().id(), value));
}

#[cfg(test)]
fn wait_after_commit_before_identity_refresh() {
    let barrier = AFTER_COMMIT_BARRIER
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("after-commit barrier is available")
        .as_ref()
        .filter(|(thread_id, _)| *thread_id == std::thread::current().id())
        .map(|(_, barrier)| std::sync::Arc::clone(barrier));
    if let Some(barrier) = barrier {
        barrier.wait();
        barrier.wait();
    }
}

/// Metadata schema 1-to-2 migration for persistent job snapshots and history.
pub const JOB_MIGRATION: &str =
    include_str!("../../../migrations/metadata-sqlite/0002_job_runtime.sql");

/// Stable, path-safe classification for persistent job failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobErrorKind {
    InvalidRequest,
    JobNotFound,
    InvalidTransition,
    RevisionConflict,
    IncompatibleSchema,
    DataIntegrity,
    Database,
    Timestamp,
}

/// Typed failures for persistent job operations.
#[derive(Debug)]
pub enum JobError {
    InvalidRequest(String),
    JobNotFound(String),
    InvalidTransition { from: String, to: String },
    RevisionConflict { expected: i64, actual: i64 },
    IncompatibleSchema { expected: i64, actual: i64 },
    DataIntegrity(String),
    Database(rusqlite::Error),
    Timestamp(String),
}

impl JobError {
    /// Return a stable category suitable for mapping into a safe IPC envelope.
    #[must_use]
    pub const fn kind(&self) -> JobErrorKind {
        match self {
            Self::InvalidRequest(_) => JobErrorKind::InvalidRequest,
            Self::JobNotFound(_) => JobErrorKind::JobNotFound,
            Self::InvalidTransition { .. } => JobErrorKind::InvalidTransition,
            Self::RevisionConflict { .. } => JobErrorKind::RevisionConflict,
            Self::IncompatibleSchema { .. } => JobErrorKind::IncompatibleSchema,
            Self::DataIntegrity(_) => JobErrorKind::DataIntegrity,
            Self::Database(_) => JobErrorKind::Database,
            Self::Timestamp(_) => JobErrorKind::Timestamp,
        }
    }
}

impl Display for JobError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(_) => formatter.write_str("invalid job request"),
            Self::JobNotFound(_) => formatter.write_str("job was not found"),
            Self::InvalidTransition { .. } => formatter.write_str("invalid job transition"),
            Self::RevisionConflict { .. } => formatter.write_str("job revision conflict"),
            Self::IncompatibleSchema { .. } => {
                formatter.write_str("metadata schema is incompatible")
            }
            Self::DataIntegrity(_) => formatter.write_str("job integrity failed"),
            Self::Database(_) => formatter.write_str("job metadata operation failed"),
            Self::Timestamp(_) => formatter.write_str("job timestamp is invalid"),
        }
    }
}

impl std::error::Error for JobError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for JobError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

/// Stable keyset cursor for descending job snapshot pagination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobListCursor {
    pub updated_at: String,
    pub job_id: String,
}

/// One bounded page of project-scoped job snapshots.
#[derive(Debug, Clone, PartialEq)]
pub struct JobPage {
    pub items: Vec<JobDescriptor>,
    pub next_cursor: Option<JobListCursor>,
}

/// Persistent, project-scoped job snapshot and history store.
#[derive(Debug)]
pub struct JobStore {
    project_path: PathBuf,
    project_id: String,
    metadata: PinnedProjectMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelling,
    Cancelled,
}

impl JobStatus {
    #[cfg(test)]
    const ALL: [Self; 6] = [
        Self::Queued,
        Self::Running,
        Self::Succeeded,
        Self::Failed,
        Self::Cancelling,
        Self::Cancelled,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Running => "RUNNING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Cancelling => "CANCELLING",
            Self::Cancelled => "CANCELLED",
        }
    }

    fn parse(value: &str) -> Result<Self, JobError> {
        match value {
            "QUEUED" => Ok(Self::Queued),
            "RUNNING" => Ok(Self::Running),
            "SUCCEEDED" => Ok(Self::Succeeded),
            "FAILED" => Ok(Self::Failed),
            "CANCELLING" => Ok(Self::Cancelling),
            "CANCELLED" => Ok(Self::Cancelled),
            _ => Err(JobError::DataIntegrity(
                "persisted job status is not recognized".to_owned(),
            )),
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

fn transition_allowed(from: JobStatus, to: JobStatus) -> bool {
    matches!(
        (from, to),
        (
            JobStatus::Queued,
            JobStatus::Running | JobStatus::Cancelling | JobStatus::Failed
        ) | (
            JobStatus::Running,
            JobStatus::Succeeded | JobStatus::Failed | JobStatus::Cancelling
        ) | (
            JobStatus::Cancelling,
            JobStatus::Cancelled | JobStatus::Failed
        )
    )
}

#[derive(Clone, Copy)]
enum TransitionRequest<'a> {
    Plain(&'a JobTransitionRequest),
    Failure(&'a JobFailureRequest),
}

impl<'a> TransitionRequest<'a> {
    fn job_id(self) -> &'a str {
        match self {
            Self::Plain(request) => &request.job_id,
            Self::Failure(request) => &request.job_id,
        }
    }

    fn correlation_id(self) -> &'a str {
        match self {
            Self::Plain(request) => &request.correlation_id,
            Self::Failure(request) => &request.correlation_id,
        }
    }

    const fn expected_revision(self) -> i64 {
        match self {
            Self::Plain(request) => request.expected_revision,
            Self::Failure(request) => request.expected_revision,
        }
    }

    const fn failure(self) -> Option<&'a JobFailureRequest> {
        match self {
            Self::Plain(_) => None,
            Self::Failure(request) => Some(request),
        }
    }
}

impl JobStore {
    /// Open a validated schema-two project without mutating its control files.
    ///
    /// # Errors
    ///
    /// Returns a typed error for unsafe layouts, incompatible schemas, invalid
    /// project identity, or database failures.
    pub fn open(project_path: &Path) -> Result<Self, JobError> {
        let layout = validated_project_layout(project_path)?;
        let metadata = pin_project_metadata(&layout).map_err(|_| {
            JobError::DataIntegrity("project metadata could not be pinned safely".to_owned())
        })?;
        let operation = begin_pinned_operation(&metadata)?;
        let descriptor = validate_schema_two_project(layout.root())?;
        let canonical_path = PathBuf::from(&descriptor.project_path);
        if canonical_path != layout.root() {
            return Err(JobError::DataIntegrity(
                "validated project path changed while opening".to_owned(),
            ));
        }
        verify_pinned_metadata(&operation)?;
        let connection = open_connection(metadata.path(), ConnectionAccess::ReadOnly)?;
        verify_pinned_metadata(&operation)?;
        probe_job_connection(&connection, &descriptor.project_id)?;
        verify_pinned_metadata(&operation)?;
        drop(connection);
        drop(operation);
        Ok(Self {
            project_path: canonical_path,
            project_id: descriptor.project_id,
            metadata,
        })
    }

    /// Persist a new queued job and matching immutable history atomically.
    ///
    /// # Errors
    ///
    /// Returns a typed error when validation, timestamp creation, or the
    /// immediate `SQLite` transaction fails.
    pub fn enqueue(&self, request: &JobEnqueueRequest) -> Result<JobDescriptor, JobError> {
        let timestamp = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|error| JobError::Timestamp(error.to_string()))?;
        self.enqueue_at(request, &timestamp)
    }

    /// Move one queued job into active execution.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, transition, concurrency, integrity, timestamp,
    /// or database failure without partially persisted history.
    pub fn start(&self, request: &JobTransitionRequest) -> Result<JobDescriptor, JobError> {
        self.start_at(request, &current_timestamp()?)
    }

    /// Mark one running job as successfully completed.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, transition, concurrency, integrity, timestamp,
    /// or database failure without partially persisted history.
    pub fn succeed(&self, request: &JobTransitionRequest) -> Result<JobDescriptor, JobError> {
        self.succeed_at(request, &current_timestamp()?)
    }

    /// Mark one queued, running, or cancelling job as safely failed.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, transition, concurrency, integrity, timestamp,
    /// or database failure without exposing raw failure details.
    pub fn fail(&self, request: &JobFailureRequest) -> Result<JobDescriptor, JobError> {
        self.fail_at(request, &current_timestamp()?)
    }

    /// Persist one bounded, monotonic progress snapshot for an active job.
    ///
    /// A phase change may reset the current amount; updates within one phase
    /// cannot regress.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, transition, concurrency, integrity, timestamp,
    /// or database failure without partially persisted history.
    pub fn update_progress(
        &self,
        request: &JobProgressUpdateRequest,
    ) -> Result<JobDescriptor, JobError> {
        self.update_progress_at(request, &current_timestamp()?)
    }

    /// Cooperatively request cancellation without claiming completion.
    ///
    /// Repeating a current cancellation request returns the unchanged snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, transition, concurrency, integrity, timestamp,
    /// or database failure without partially persisted history.
    pub fn request_cancellation(
        &self,
        request: &JobTransitionRequest,
    ) -> Result<JobDescriptor, JobError> {
        self.request_cancellation_at(request, &current_timestamp()?)
    }

    /// Complete a previously acknowledged cooperative cancellation.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, transition, concurrency, integrity, timestamp,
    /// or database failure without partially persisted history.
    pub fn complete_cancellation(
        &self,
        request: &JobTransitionRequest,
    ) -> Result<JobDescriptor, JobError> {
        self.complete_cancellation_at(request, &current_timestamp()?)
    }

    /// Mark active jobs left by an interrupted process as retriable failures.
    ///
    /// Queued and terminal jobs remain unchanged. Repeating recovery after a
    /// successful pass writes no duplicate history.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, integrity, timestamp, or database failure;
    /// the complete recovery batch rolls back when any write fails.
    pub fn recover_interrupted(
        &self,
        correlation_id: &str,
    ) -> Result<Vec<JobDescriptor>, JobError> {
        self.recover_interrupted_at(correlation_id, &current_timestamp()?)
    }

    /// Return one safe job snapshot by lowercase UUID v7 identity.
    ///
    /// # Errors
    ///
    /// Returns `InvalidRequest`, `JobNotFound`, or a safe database failure.
    pub fn get(&self, job_id: &str) -> Result<JobDescriptor, JobError> {
        validate_uuid(job_id, "job_id")?;
        let operation = begin_pinned_operation(&self.metadata)?;
        let connection = self.read_connection(&operation)?;
        let decoded = connection
            .query_row(
                "SELECT job_id, project_id, kind, status, correlation_id, revision,
                        created_at, started_at, finished_at, updated_at,
                        progress_current, progress_total, progress_unit, progress_phase,
                        progress_message, error_code, error_message, error_retriable
                 FROM job
                 WHERE project_id = ?1 AND job_id = ?2",
                params![self.project_id, job_id],
                row_to_descriptor,
            )
            .optional()?;
        decoded
            .map(|row| validate_persisted_descriptor(row.descriptor, row.error_retriable))
            .transpose()?
            .ok_or_else(|| JobError::JobNotFound("job identity does not exist".to_owned()))
    }

    /// List one deterministic bounded page ordered newest-first.
    ///
    /// # Errors
    ///
    /// Returns `InvalidRequest` for a page size outside `1..=100` or an
    /// invalid cursor, and a safe database failure for query errors.
    pub fn list(&self, limit: usize, cursor: Option<JobListCursor>) -> Result<JobPage, JobError> {
        if !(1..=100).contains(&limit) {
            return Err(JobError::InvalidRequest(
                "page size must be between 1 and 100".to_owned(),
            ));
        }
        if let Some(value) = cursor.as_ref() {
            validate_timestamp(&value.updated_at).map_err(|_| {
                JobError::InvalidRequest("cursor timestamp must be UTC RFC 3339".to_owned())
            })?;
            validate_uuid(&value.job_id, "cursor job_id")?;
        }
        let fetch_limit = i64::try_from(limit + 1).map_err(|_| {
            JobError::InvalidRequest("page size cannot be represented safely".to_owned())
        })?;
        let operation = begin_pinned_operation(&self.metadata)?;
        let connection = self.read_connection(&operation)?;
        let decoded = if let Some(value) = cursor {
            let mut statement = connection.prepare(
                "SELECT job_id, project_id, kind, status, correlation_id, revision,
                        created_at, started_at, finished_at, updated_at,
                        progress_current, progress_total, progress_unit, progress_phase,
                        progress_message, error_code, error_message, error_retriable
                 FROM job
                 WHERE project_id = ?1
                   AND (updated_at < ?2 OR (updated_at = ?2 AND job_id < ?3))
                 ORDER BY updated_at DESC, job_id DESC
                 LIMIT ?4",
            )?;
            let collected = statement
                .query_map(
                    params![self.project_id, value.updated_at, value.job_id, fetch_limit],
                    row_to_descriptor,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            collected
        } else {
            let mut statement = connection.prepare(
                "SELECT job_id, project_id, kind, status, correlation_id, revision,
                        created_at, started_at, finished_at, updated_at,
                        progress_current, progress_total, progress_unit, progress_phase,
                        progress_message, error_code, error_message, error_retriable
                 FROM job
                 WHERE project_id = ?1
                 ORDER BY updated_at DESC, job_id DESC
                 LIMIT ?2",
            )?;
            let collected = statement
                .query_map(params![self.project_id, fetch_limit], row_to_descriptor)?
                .collect::<Result<Vec<_>, _>>()?;
            collected
        };
        let mut items = decoded
            .into_iter()
            .map(|row| validate_persisted_descriptor(row.descriptor, row.error_retriable))
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = items.len() > limit;
        if has_more {
            items.truncate(limit);
        }
        let next_cursor = if has_more {
            let last = items.last().ok_or_else(|| {
                JobError::DataIntegrity("bounded list returned an invalid page".to_owned())
            })?;
            Some(JobListCursor {
                updated_at: last.updated_at.clone(),
                job_id: last.job_id.clone(),
            })
        } else {
            None
        };
        Ok(JobPage { items, next_cursor })
    }

    fn enqueue_at(
        &self,
        request: &JobEnqueueRequest,
        timestamp: &str,
    ) -> Result<JobDescriptor, JobError> {
        validate_enqueue(request)?;
        validate_timestamp(timestamp)?;
        let job_event_id = event_id_at("job-event", &request.job_id, 1, "job.queued", timestamp)?;
        let audit_event_id =
            event_id_at("audit-event", &request.job_id, 1, "job.queued", timestamp)?;
        let descriptor = JobDescriptor {
            correlation_id: request.correlation_id.clone(),
            created_at: timestamp.to_owned(),
            job_id: request.job_id.clone(),
            kind: request.kind.clone(),
            progress_current: 0,
            project_id: self.project_id.clone(),
            revision: 1,
            status: "QUEUED".to_owned(),
            updated_at: timestamp.to_owned(),
            error_code: None,
            error_message: None,
            error_retriable: None,
            finished_at: None,
            progress_message: None,
            progress_phase: None,
            progress_total: request.progress_total,
            progress_unit: request.progress_unit.clone(),
            started_at: None,
        };
        let descriptor = validate_persisted_descriptor(descriptor, None)?;
        let after_hash = snapshot_hash(&descriptor)?;

        let mut metadata_operation = begin_pinned_operation(&self.metadata)?;
        let mut connection = self.write_connection(&metadata_operation)?;
        verify_pinned_metadata(&metadata_operation)?;
        let write_result = (|| -> Result<(), JobError> {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute(
                "INSERT INTO job (
                    job_id, project_id, kind, status, correlation_id, revision,
                    created_at, updated_at, progress_current, progress_total, progress_unit
                 ) VALUES (?1, ?2, ?3, 'QUEUED', ?4, 1, ?5, ?5, 0, ?6, ?7)",
                params![
                    descriptor.job_id,
                    descriptor.project_id,
                    descriptor.kind,
                    descriptor.correlation_id,
                    descriptor.created_at,
                    descriptor.progress_total,
                    descriptor.progress_unit,
                ],
            )?;
            transaction.execute(
                "INSERT INTO job_event (
                    event_id, job_id, event_type, from_status, to_status, revision,
                    progress_current, progress_total, progress_unit, occurred_at, correlation_id
                 ) VALUES (?1, ?2, 'job.queued', NULL, 'QUEUED', 1, 0, ?3, ?4, ?5, ?6)",
                params![
                    job_event_id,
                    descriptor.job_id,
                    descriptor.progress_total,
                    descriptor.progress_unit,
                    descriptor.created_at,
                    descriptor.correlation_id,
                ],
            )?;
            transaction.execute(
                "INSERT INTO audit_event (
                    event_id, actor, action, target_type, target_id,
                    before_hash, after_hash, occurred_at, correlation_id
                 ) VALUES (?1, 'local-user', 'job.queued', 'job', ?2, NULL, ?3, ?4, ?5)",
                params![
                    audit_event_id,
                    descriptor.job_id,
                    after_hash,
                    descriptor.created_at,
                    descriptor.correlation_id,
                ],
            )?;
            transaction.commit()?;
            Ok(())
        })();
        #[cfg(test)]
        if write_result.is_ok() {
            wait_after_commit_before_identity_refresh();
        }
        let refresh_result = metadata_operation
            .refresh_after_authorized_write()
            .map_err(|_| JobError::DataIntegrity("pinned metadata identity changed".to_owned()));
        refresh_result?;
        write_result?;
        Ok(descriptor)
    }

    fn start_at(
        &self,
        request: &JobTransitionRequest,
        timestamp: &str,
    ) -> Result<JobDescriptor, JobError> {
        self.transition_to_at(
            TransitionRequest::Plain(request),
            JobStatus::Running,
            timestamp,
        )
    }

    fn succeed_at(
        &self,
        request: &JobTransitionRequest,
        timestamp: &str,
    ) -> Result<JobDescriptor, JobError> {
        self.transition_to_at(
            TransitionRequest::Plain(request),
            JobStatus::Succeeded,
            timestamp,
        )
    }

    fn fail_at(
        &self,
        request: &JobFailureRequest,
        timestamp: &str,
    ) -> Result<JobDescriptor, JobError> {
        self.transition_to_at(
            TransitionRequest::Failure(request),
            JobStatus::Failed,
            timestamp,
        )
    }

    fn update_progress_at(
        &self,
        request: &JobProgressUpdateRequest,
        timestamp: &str,
    ) -> Result<JobDescriptor, JobError> {
        validate_progress_request(request)?;
        validate_timestamp(timestamp)?;
        self.with_immediate_transaction(|transaction| {
            let before = select_job(transaction, &self.project_id, &request.job_id)?;
            if before.revision != request.expected_revision {
                return Err(JobError::RevisionConflict {
                    expected: request.expected_revision,
                    actual: before.revision,
                });
            }
            let status = JobStatus::parse(&before.status)?;
            if !matches!(status, JobStatus::Running | JobStatus::Cancelling) {
                return Err(JobError::InvalidTransition {
                    from: status.as_str().to_owned(),
                    to: "PROGRESS".to_owned(),
                });
            }
            if before.progress_phase.as_deref() == Some(request.phase.as_str())
                && request.current < before.progress_current
            {
                return Err(JobError::InvalidRequest(
                    "progress cannot regress within the same phase".to_owned(),
                ));
            }
            ensure_timestamp_not_earlier(timestamp, &before.updated_at)?;
            let revision = before
                .revision
                .checked_add(1)
                .ok_or_else(|| JobError::DataIntegrity("job revision overflowed".to_owned()))?;
            let after = JobDescriptor {
                correlation_id: request.correlation_id.clone(),
                created_at: before.created_at.clone(),
                job_id: before.job_id.clone(),
                kind: before.kind.clone(),
                progress_current: request.current,
                project_id: before.project_id.clone(),
                revision,
                status: before.status.clone(),
                updated_at: timestamp.to_owned(),
                error_code: None,
                error_message: None,
                error_retriable: None,
                finished_at: None,
                progress_message: Some(request.message.clone()),
                progress_phase: Some(request.phase.clone()),
                progress_total: request.total,
                progress_unit: request.unit.clone(),
                started_at: before.started_at.clone(),
            };
            let raw_error_retriable = after.error_retriable.map(i64::from);
            let after = validate_persisted_descriptor(after, raw_error_retriable)?;
            persist_mutation(
                transaction,
                &self.project_id,
                &before,
                &after,
                "job.progressed",
            )?;
            Ok(after)
        })
    }

    fn request_cancellation_at(
        &self,
        request: &JobTransitionRequest,
        timestamp: &str,
    ) -> Result<JobDescriptor, JobError> {
        self.transition_to_at_internal(
            TransitionRequest::Plain(request),
            JobStatus::Cancelling,
            timestamp,
            true,
        )
    }

    fn complete_cancellation_at(
        &self,
        request: &JobTransitionRequest,
        timestamp: &str,
    ) -> Result<JobDescriptor, JobError> {
        self.transition_to_at(
            TransitionRequest::Plain(request),
            JobStatus::Cancelled,
            timestamp,
        )
    }

    fn recover_interrupted_at(
        &self,
        correlation_id: &str,
        timestamp: &str,
    ) -> Result<Vec<JobDescriptor>, JobError> {
        validate_uuid(correlation_id, "correlation_id")?;
        validate_timestamp(timestamp)?;
        self.with_immediate_transaction(|transaction| {
            let active = {
                let mut statement = transaction.prepare(
                    "SELECT job_id, project_id, kind, status, correlation_id, revision,
                            created_at, started_at, finished_at, updated_at,
                            progress_current, progress_total, progress_unit, progress_phase,
                            progress_message, error_code, error_message, error_retriable
                     FROM job
                     WHERE project_id = ?1 AND status IN ('RUNNING', 'CANCELLING')
                     ORDER BY job_id ASC",
                )?;
                let jobs = statement
                    .query_map([&self.project_id], row_to_descriptor)?
                    .collect::<Result<Vec<_>, _>>()?;
                jobs.into_iter()
                    .map(|row| validate_persisted_descriptor(row.descriptor, row.error_retriable))
                    .collect::<Result<Vec<_>, _>>()?
            };
            let mut recovered = Vec::with_capacity(active.len());
            for before in active {
                ensure_timestamp_not_earlier(timestamp, &before.updated_at)?;
                let revision = before
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| JobError::DataIntegrity("job revision overflowed".to_owned()))?;
                let after = JobDescriptor {
                    correlation_id: correlation_id.to_owned(),
                    created_at: before.created_at.clone(),
                    job_id: before.job_id.clone(),
                    kind: before.kind.clone(),
                    progress_current: before.progress_current,
                    project_id: before.project_id.clone(),
                    revision,
                    status: JobStatus::Failed.as_str().to_owned(),
                    updated_at: timestamp.to_owned(),
                    error_code: Some("INTERRUPTED".to_owned()),
                    error_message: Some(
                        "Pekerjaan terhenti saat aplikasi tidak aktif dan dapat dicoba kembali."
                            .to_owned(),
                    ),
                    error_retriable: Some(true),
                    finished_at: Some(timestamp.to_owned()),
                    progress_message: before.progress_message.clone(),
                    progress_phase: before.progress_phase.clone(),
                    progress_total: before.progress_total,
                    progress_unit: before.progress_unit.clone(),
                    started_at: before.started_at.clone(),
                };
                let raw_error_retriable = after.error_retriable.map(i64::from);
                let after = validate_persisted_descriptor(after, raw_error_retriable)?;
                persist_mutation(
                    transaction,
                    &self.project_id,
                    &before,
                    &after,
                    "job.interrupted",
                )?;
                recovered.push(after);
            }
            Ok(recovered)
        })
    }

    fn transition_to_at(
        &self,
        request: TransitionRequest<'_>,
        to: JobStatus,
        timestamp: &str,
    ) -> Result<JobDescriptor, JobError> {
        self.transition_to_at_internal(request, to, timestamp, false)
    }

    fn transition_to_at_internal(
        &self,
        request: TransitionRequest<'_>,
        to: JobStatus,
        timestamp: &str,
        idempotent_cancelling: bool,
    ) -> Result<JobDescriptor, JobError> {
        validate_transition_request(request, to)?;
        validate_timestamp(timestamp)?;
        self.with_immediate_transaction(|transaction| {
            let before = select_job(transaction, &self.project_id, request.job_id())?;
            if before.revision != request.expected_revision() {
                return Err(JobError::RevisionConflict {
                    expected: request.expected_revision(),
                    actual: before.revision,
                });
            }
            let from = JobStatus::parse(&before.status)?;
            if idempotent_cancelling && from == JobStatus::Cancelling && to == JobStatus::Cancelling
            {
                return Ok(before);
            }
            if !transition_allowed(from, to) {
                return Err(JobError::InvalidTransition {
                    from: from.as_str().to_owned(),
                    to: to.as_str().to_owned(),
                });
            }
            ensure_timestamp_not_earlier(timestamp, &before.updated_at)?;
            let after = transitioned_descriptor(&before, request, from, to, timestamp)?;
            let action = transition_action(from, to)?;
            persist_mutation(transaction, &self.project_id, &before, &after, action)?;
            Ok(after)
        })
    }

    fn with_immediate_transaction<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T, JobError>,
    ) -> Result<T, JobError> {
        let mut metadata_operation = begin_pinned_operation(&self.metadata)?;
        let mut connection = self.write_connection(&metadata_operation)?;
        verify_pinned_metadata(&metadata_operation)?;
        let write_result = (|| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let value = operation(&transaction)?;
            transaction.commit()?;
            Ok(value)
        })();
        #[cfg(test)]
        if write_result.is_ok() {
            wait_after_commit_before_identity_refresh();
        }
        metadata_operation
            .refresh_after_authorized_write()
            .map_err(|_| JobError::DataIntegrity("pinned metadata identity changed".to_owned()))?;
        write_result
    }

    fn read_connection(
        &self,
        operation: &PinnedProjectOperation<'_>,
    ) -> Result<Connection, JobError> {
        self.validated_connection(operation, ConnectionAccess::ReadOnly)
    }

    fn write_connection(
        &self,
        operation: &PinnedProjectOperation<'_>,
    ) -> Result<Connection, JobError> {
        self.validated_connection(operation, ConnectionAccess::ReadWrite)
    }

    fn validated_connection(
        &self,
        operation: &PinnedProjectOperation<'_>,
        access: ConnectionAccess,
    ) -> Result<Connection, JobError> {
        let layout = validated_project_layout(&self.project_path)?;
        if layout.root() != self.project_path || layout.metadata_path() != self.metadata.path() {
            return Err(JobError::DataIntegrity(
                "project control paths changed after the job store opened".to_owned(),
            ));
        }
        verify_pinned_metadata(operation)?;
        let connection = open_connection(self.metadata.path(), access)?;
        verify_pinned_metadata(operation)?;
        probe_job_connection(&connection, &self.project_id)?;
        verify_pinned_metadata(operation)?;
        Ok(connection)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionAccess {
    ReadOnly,
    ReadWrite,
}

fn validate_schema_two_project(
    project_path: &Path,
) -> Result<teratai_contracts::generated::project_descriptor::ProjectDescriptor, JobError> {
    #[cfg(test)]
    FULL_PROJECT_VALIDATIONS.with(|count| count.set(count.get() + 1));
    let descriptor = crate::ProjectService::open(project_path)
        .map_err(|_| JobError::DataIntegrity("full project validation failed".to_owned()))?;
    if descriptor.metadata_schema_version != 2 {
        return Err(JobError::IncompatibleSchema {
            expected: 2,
            actual: descriptor.metadata_schema_version,
        });
    }
    Ok(descriptor)
}

#[cfg(test)]
thread_local! {
    static FULL_PROJECT_VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn full_project_validation_count() -> usize {
    FULL_PROJECT_VALIDATIONS.with(std::cell::Cell::get)
}

fn validated_project_layout(project_path: &Path) -> Result<ProjectLayout, JobError> {
    validate_project_layout(project_path)
        .map_err(|_| JobError::DataIntegrity("project control layout is unsafe".to_owned()))
}

fn begin_pinned_operation(
    metadata: &PinnedProjectMetadata,
) -> Result<PinnedProjectOperation<'_>, JobError> {
    metadata
        .begin_operation()
        .map_err(|_| JobError::DataIntegrity("pinned metadata identity changed".to_owned()))
}

fn verify_pinned_metadata(operation: &PinnedProjectOperation<'_>) -> Result<(), JobError> {
    operation
        .verify()
        .map_err(|_| JobError::DataIntegrity("pinned metadata identity changed".to_owned()))
}

fn open_connection(path: &Path, access: ConnectionAccess) -> Result<Connection, JobError> {
    let access_flag = match access {
        ConnectionAccess::ReadOnly => OpenFlags::SQLITE_OPEN_READ_ONLY,
        ConnectionAccess::ReadWrite => OpenFlags::SQLITE_OPEN_READ_WRITE,
    };
    let connection =
        Connection::open_with_flags(path, access_flag | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    connection.pragma_update(None, "foreign_keys", true)?;
    Ok(connection)
}

fn probe_job_schema(connection: &Connection) -> Result<(), JobError> {
    let object_count: i64 = connection.query_row(
        "SELECT COUNT(*)
         FROM sqlite_schema
         WHERE (type = 'table' AND name IN ('job', 'job_event'))
            OR (type = 'trigger' AND name IN (
                'job_event_prevent_update', 'job_event_prevent_delete'
            ))
            OR (type = 'index' AND name IN (
                'job_status_updated_idx', 'job_correlation_idx',
                'job_event_job_sequence_idx', 'job_event_correlation_sequence_idx'
            ))",
        [],
        |row| row.get(0),
    )?;
    if object_count != 8 {
        return Err(JobError::DataIntegrity(
            "required job schema objects are missing".to_owned(),
        ));
    }
    connection
        .prepare(
            "SELECT job_id, project_id, kind, status, correlation_id, revision,
                    created_at, started_at, finished_at, updated_at,
                    progress_current, progress_total, progress_unit, progress_phase,
                    progress_message, error_code, error_message, error_retriable
             FROM job LIMIT 0",
        )
        .map_err(|_| JobError::DataIntegrity("job snapshot schema is invalid".to_owned()))?;
    connection
        .prepare(
            "SELECT sequence, event_id, job_id, event_type, from_status, to_status,
                    revision, progress_current, progress_total, progress_unit,
                    progress_phase, progress_message, error_code, error_message,
                    error_retriable, occurred_at, correlation_id
             FROM job_event LIMIT 0",
        )
        .map_err(|_| JobError::DataIntegrity("job event schema is invalid".to_owned()))?;
    Ok(())
}

fn probe_job_connection(connection: &Connection, project_id: &str) -> Result<(), JobError> {
    let schema_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema_version != 2 {
        return Err(JobError::IncompatibleSchema {
            expected: 2,
            actual: schema_version,
        });
    }
    probe_job_schema(connection)?;
    let connected_project_id: String = connection.query_row(
        "SELECT project_id FROM project WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    if connected_project_id != project_id {
        return Err(JobError::DataIntegrity(
            "metadata identity changed during connection validation".to_owned(),
        ));
    }
    Ok(())
}

fn current_timestamp() -> Result<String, JobError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| JobError::Timestamp(error.to_string()))
}

fn validate_transition_request(
    request: TransitionRequest<'_>,
    to: JobStatus,
) -> Result<(), JobError> {
    validate_uuid(request.job_id(), "job_id")?;
    validate_uuid(request.correlation_id(), "correlation_id")?;
    if request.expected_revision() <= 0 {
        return Err(JobError::InvalidRequest(
            "expected_revision must be positive".to_owned(),
        ));
    }
    match (to, request.failure()) {
        (JobStatus::Failed, Some(failure)) => validate_failure(failure),
        (JobStatus::Failed, None) => Err(JobError::InvalidRequest(
            "failure transition requires safe failure details".to_owned(),
        )),
        (_, Some(_)) => Err(JobError::InvalidRequest(
            "failure details are only valid for failed jobs".to_owned(),
        )),
        (_, None) => Ok(()),
    }
}

fn validate_failure(request: &JobFailureRequest) -> Result<(), JobError> {
    if !is_safe_error_code(&request.error_code) {
        return Err(JobError::InvalidRequest(
            "error_code must be a bounded uppercase identifier".to_owned(),
        ));
    }
    if !is_safe_message(&request.error_message, 500) {
        return Err(JobError::InvalidRequest(
            "error_message must be bounded safe text".to_owned(),
        ));
    }
    Ok(())
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

fn ensure_timestamp_not_earlier(timestamp: &str, previous: &str) -> Result<(), JobError> {
    let next = validate_timestamp(timestamp)?;
    let prior = validate_timestamp(previous)?;
    if next < prior {
        return Err(JobError::InvalidRequest(
            "mutation timestamp cannot precede the persisted snapshot".to_owned(),
        ));
    }
    Ok(())
}

fn select_job(
    transaction: &Transaction<'_>,
    project_id: &str,
    job_id: &str,
) -> Result<JobDescriptor, JobError> {
    let decoded = transaction
        .query_row(
            "SELECT job_id, project_id, kind, status, correlation_id, revision,
                    created_at, started_at, finished_at, updated_at,
                    progress_current, progress_total, progress_unit, progress_phase,
                    progress_message, error_code, error_message, error_retriable
             FROM job
             WHERE project_id = ?1 AND job_id = ?2",
            params![project_id, job_id],
            row_to_descriptor,
        )
        .optional()?;
    decoded
        .map(|row| validate_persisted_descriptor(row.descriptor, row.error_retriable))
        .transpose()?
        .ok_or_else(|| JobError::JobNotFound("job identity does not exist".to_owned()))
}

fn transitioned_descriptor(
    before: &JobDescriptor,
    request: TransitionRequest<'_>,
    from: JobStatus,
    to: JobStatus,
    timestamp: &str,
) -> Result<JobDescriptor, JobError> {
    let revision = before
        .revision
        .checked_add(1)
        .ok_or_else(|| JobError::DataIntegrity("job revision overflowed".to_owned()))?;
    let failure = request.failure();
    let descriptor = JobDescriptor {
        correlation_id: request.correlation_id().to_owned(),
        created_at: before.created_at.clone(),
        job_id: before.job_id.clone(),
        kind: before.kind.clone(),
        progress_current: before.progress_current,
        project_id: before.project_id.clone(),
        revision,
        status: to.as_str().to_owned(),
        updated_at: timestamp.to_owned(),
        error_code: failure.map(|value| value.error_code.clone()),
        error_message: failure.map(|value| value.error_message.clone()),
        error_retriable: failure.map(|value| value.error_retriable),
        finished_at: to.is_terminal().then(|| timestamp.to_owned()),
        progress_message: before.progress_message.clone(),
        progress_phase: before.progress_phase.clone(),
        progress_total: before.progress_total,
        progress_unit: before.progress_unit.clone(),
        started_at: if from == JobStatus::Queued && to == JobStatus::Running {
            Some(timestamp.to_owned())
        } else {
            before.started_at.clone()
        },
    };
    let raw_error_retriable = descriptor.error_retriable.map(i64::from);
    validate_persisted_descriptor(descriptor, raw_error_retriable)
}

fn transition_action(from: JobStatus, to: JobStatus) -> Result<&'static str, JobError> {
    match (from, to) {
        (JobStatus::Queued, JobStatus::Running) => Ok("job.started"),
        (JobStatus::Running, JobStatus::Succeeded) => Ok("job.succeeded"),
        (JobStatus::Queued | JobStatus::Running, JobStatus::Cancelling) => {
            Ok("job.cancellation_requested")
        }
        (JobStatus::Cancelling, JobStatus::Cancelled) => Ok("job.cancelled"),
        (JobStatus::Queued | JobStatus::Running | JobStatus::Cancelling, JobStatus::Failed) => {
            Ok("job.failed")
        }
        _ => Err(JobError::DataIntegrity(
            "transition action is not defined".to_owned(),
        )),
    }
}

fn persist_mutation(
    transaction: &Transaction<'_>,
    project_id: &str,
    before: &JobDescriptor,
    after: &JobDescriptor,
    action: &str,
) -> Result<(), JobError> {
    let changed = transaction.execute(
        "UPDATE job
         SET status = ?1, correlation_id = ?2, revision = ?3,
             started_at = ?4, finished_at = ?5, updated_at = ?6,
             progress_current = ?7, progress_total = ?8, progress_unit = ?9,
             progress_phase = ?10, progress_message = ?11,
             error_code = ?12, error_message = ?13, error_retriable = ?14
         WHERE project_id = ?15 AND job_id = ?16 AND revision = ?17",
        params![
            after.status,
            after.correlation_id,
            after.revision,
            after.started_at,
            after.finished_at,
            after.updated_at,
            after.progress_current,
            after.progress_total,
            after.progress_unit,
            after.progress_phase,
            after.progress_message,
            after.error_code,
            after.error_message,
            after.error_retriable.map(i64::from),
            project_id,
            after.job_id,
            before.revision,
        ],
    )?;
    if changed != 1 {
        return Err(JobError::RevisionConflict {
            expected: before.revision,
            actual: before.revision,
        });
    }
    let job_event_id = event_id_at(
        "job-event",
        &after.job_id,
        after.revision,
        action,
        &after.updated_at,
    )?;
    let audit_event_id = event_id_at(
        "audit-event",
        &after.job_id,
        after.revision,
        action,
        &after.updated_at,
    )?;
    transaction.execute(
        "INSERT INTO job_event (
            event_id, job_id, event_type, from_status, to_status, revision,
            progress_current, progress_total, progress_unit, progress_phase,
            progress_message, error_code, error_message, error_retriable,
            occurred_at, correlation_id
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
         )",
        params![
            job_event_id,
            after.job_id,
            action,
            before.status,
            after.status,
            after.revision,
            after.progress_current,
            after.progress_total,
            after.progress_unit,
            after.progress_phase,
            after.progress_message,
            after.error_code,
            after.error_message,
            after.error_retriable.map(i64::from),
            after.updated_at,
            after.correlation_id,
        ],
    )?;
    let before_hash = snapshot_hash(before)?;
    let after_hash = snapshot_hash(after)?;
    transaction.execute(
        "INSERT INTO audit_event (
            event_id, actor, action, target_type, target_id,
            before_hash, after_hash, occurred_at, correlation_id
         ) VALUES (?1, 'local-user', ?2, 'job', ?3, ?4, ?5, ?6, ?7)",
        params![
            audit_event_id,
            action,
            after.job_id,
            before_hash,
            after_hash,
            after.updated_at,
            after.correlation_id,
        ],
    )?;
    Ok(())
}

fn validate_progress_request(request: &JobProgressUpdateRequest) -> Result<(), JobError> {
    validate_uuid(&request.job_id, "job_id")?;
    validate_uuid(&request.correlation_id, "correlation_id")?;
    if request.expected_revision <= 0 {
        return Err(JobError::InvalidRequest(
            "expected_revision must be positive".to_owned(),
        ));
    }
    if request.current < 0 {
        return Err(JobError::InvalidRequest(
            "progress current cannot be negative".to_owned(),
        ));
    }
    if request.total.is_some_and(|total| total <= 0) {
        return Err(JobError::InvalidRequest(
            "progress total must be positive when present".to_owned(),
        ));
    }
    if request.total.is_some_and(|total| request.current > total) {
        return Err(JobError::InvalidRequest(
            "progress current cannot exceed total".to_owned(),
        ));
    }
    if !is_safe_token(&request.phase, 120) {
        return Err(JobError::InvalidRequest(
            "progress phase must be a bounded safe identifier".to_owned(),
        ));
    }
    if !is_safe_message(&request.message, 500) {
        return Err(JobError::InvalidRequest(
            "progress message must be bounded safe text".to_owned(),
        ));
    }
    if request
        .unit
        .as_deref()
        .is_some_and(|unit| !is_safe_token(unit, 32))
    {
        return Err(JobError::InvalidRequest(
            "progress unit must be a bounded safe identifier".to_owned(),
        ));
    }
    Ok(())
}

fn validate_enqueue(request: &JobEnqueueRequest) -> Result<(), JobError> {
    validate_uuid(&request.job_id, "job_id")?;
    validate_uuid(&request.correlation_id, "correlation_id")?;
    if !is_safe_token(&request.kind, 120) {
        return Err(JobError::InvalidRequest(
            "kind must match [a-z][a-z0-9._-]{0,119}".to_owned(),
        ));
    }
    if request.progress_total.is_some_and(|total| total <= 0) {
        return Err(JobError::InvalidRequest(
            "progress_total must be positive when present".to_owned(),
        ));
    }
    if request
        .progress_unit
        .as_deref()
        .is_some_and(|unit| !is_safe_token(unit, 32))
    {
        return Err(JobError::InvalidRequest(
            "progress_unit must be a bounded safe identifier".to_owned(),
        ));
    }
    Ok(())
}

fn validate_uuid(value: &str, field: &str) -> Result<(), JobError> {
    if crate::is_uuid_v7(value) {
        Ok(())
    } else {
        Err(JobError::InvalidRequest(format!(
            "{field} must be a lowercase UUID v7"
        )))
    }
}

fn validate_timestamp(value: &str) -> Result<OffsetDateTime, JobError> {
    if !value.ends_with('Z') {
        return Err(JobError::Timestamp(
            "timestamp must use UTC Z notation".to_owned(),
        ));
    }
    OffsetDateTime::parse(value, &Rfc3339).map_err(|error| JobError::Timestamp(error.to_string()))
}

fn is_safe_token(value: &str, maximum: usize) -> bool {
    let bytes = value.as_bytes();
    (1..=maximum).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..].iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn event_id_at(
    label: &str,
    job_id: &str,
    revision: i64,
    event_type: &str,
    timestamp: &str,
) -> Result<String, JobError> {
    let parsed = validate_timestamp(timestamp)?;
    let milliseconds = parsed.unix_timestamp_nanos() / 1_000_000;
    let milliseconds = u64::try_from(milliseconds)
        .map_err(|_| JobError::Timestamp("timestamp precedes the UUID v7 Unix epoch".to_owned()))?;
    if milliseconds > 0x0000_ffff_ffff_ffff {
        return Err(JobError::Timestamp(
            "timestamp exceeds the UUID v7 range".to_owned(),
        ));
    }
    let mut digest = Sha256::new();
    digest.update(label.as_bytes());
    digest.update([0]);
    digest.update(job_id.as_bytes());
    digest.update([0]);
    digest.update(revision.to_be_bytes());
    digest.update([0]);
    digest.update(event_type.as_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes[..6].copy_from_slice(&milliseconds.to_be_bytes()[2..]);
    bytes[6..].copy_from_slice(&digest[..10]);
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format_uuid(bytes))
}

fn snapshot_hash(descriptor: &JobDescriptor) -> Result<String, JobError> {
    let canonical = serde_json::to_vec(descriptor).map_err(|_| {
        JobError::DataIntegrity("job snapshot could not be hashed safely".to_owned())
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(canonical)))
}

fn format_uuid(bytes: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(36);
    for (index, byte) in bytes.into_iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            result.push('-');
        }
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

struct DecodedJobDescriptor {
    descriptor: JobDescriptor,
    error_retriable: Option<i64>,
}

fn row_to_descriptor(row: &Row<'_>) -> rusqlite::Result<DecodedJobDescriptor> {
    Ok(DecodedJobDescriptor {
        descriptor: JobDescriptor {
            job_id: row.get(0)?,
            project_id: row.get(1)?,
            kind: row.get(2)?,
            status: row.get(3)?,
            correlation_id: row.get(4)?,
            revision: row.get(5)?,
            created_at: row.get(6)?,
            started_at: row.get(7)?,
            finished_at: row.get(8)?,
            updated_at: row.get(9)?,
            progress_current: row.get(10)?,
            progress_total: row.get(11)?,
            progress_unit: row.get(12)?,
            progress_phase: row.get(13)?,
            progress_message: row.get(14)?,
            error_code: row.get(15)?,
            error_message: row.get(16)?,
            error_retriable: None,
        },
        error_retriable: row.get(17)?,
    })
}

fn validate_persisted_descriptor(
    mut descriptor: JobDescriptor,
    raw_error_retriable: Option<i64>,
) -> Result<JobDescriptor, JobError> {
    descriptor.error_retriable = decode_persisted_boolean(raw_error_retriable)?;
    let status = validate_persisted_identity(&descriptor)?;
    validate_persisted_timeline(&descriptor)?;
    validate_persisted_payload(&descriptor, status)?;
    validate_persisted_status_fields(&descriptor, status)?;
    Ok(descriptor)
}

fn decode_persisted_boolean(value: Option<i64>) -> Result<Option<bool>, JobError> {
    Ok(match value {
        None => None,
        Some(0) => Some(false),
        Some(1) => Some(true),
        Some(_) => {
            return Err(persisted_integrity(
                "error_retriable is not a SQLite boolean",
            ))
        }
    })
}

fn validate_persisted_identity(descriptor: &JobDescriptor) -> Result<JobStatus, JobError> {
    for (field, value) in [
        ("job_id", descriptor.job_id.as_str()),
        ("project_id", descriptor.project_id.as_str()),
        ("correlation_id", descriptor.correlation_id.as_str()),
    ] {
        if !crate::is_uuid_v7(value) {
            return Err(persisted_integrity(&format!(
                "{field} is not a lowercase UUID v7"
            )));
        }
    }
    if !is_safe_token(&descriptor.kind, 120) {
        return Err(persisted_integrity("kind is not a bounded safe identifier"));
    }
    let status = JobStatus::parse(&descriptor.status)?;
    if descriptor.revision <= 0 {
        return Err(persisted_integrity("revision is not positive"));
    }
    Ok(status)
}

fn validate_persisted_timeline(descriptor: &JobDescriptor) -> Result<(), JobError> {
    let created_at = persisted_timestamp(&descriptor.created_at, "created_at")?;
    let updated_at = persisted_timestamp(&descriptor.updated_at, "updated_at")?;
    if updated_at < created_at {
        return Err(persisted_integrity("updated_at precedes created_at"));
    }
    let started_at = descriptor
        .started_at
        .as_deref()
        .map(|value| persisted_timestamp(value, "started_at"))
        .transpose()?;
    let finished_at = descriptor
        .finished_at
        .as_deref()
        .map(|value| persisted_timestamp(value, "finished_at"))
        .transpose()?;
    if started_at.is_some_and(|value| value < created_at || value > updated_at) {
        return Err(persisted_integrity(
            "started_at is outside the persisted job timeline",
        ));
    }
    if finished_at.is_some_and(|value| value < created_at || value > updated_at) {
        return Err(persisted_integrity(
            "finished_at is outside the persisted job timeline",
        ));
    }
    if started_at
        .zip(finished_at)
        .is_some_and(|(started, finished)| finished < started)
    {
        return Err(persisted_integrity("finished_at precedes started_at"));
    }
    Ok(())
}

fn validate_persisted_payload(
    descriptor: &JobDescriptor,
    status: JobStatus,
) -> Result<(), JobError> {
    if descriptor.progress_current < 0 {
        return Err(persisted_integrity("progress_current is negative"));
    }
    if descriptor.progress_total.is_some_and(|total| total <= 0) {
        return Err(persisted_integrity("progress_total is not positive"));
    }
    if descriptor
        .progress_total
        .is_some_and(|total| descriptor.progress_current > total)
    {
        return Err(persisted_integrity(
            "progress_current exceeds progress_total",
        ));
    }
    if descriptor
        .progress_unit
        .as_deref()
        .is_some_and(|value| !is_safe_token(value, 32))
    {
        return Err(persisted_integrity("progress_unit is unsafe"));
    }
    if descriptor
        .progress_phase
        .as_deref()
        .is_some_and(|value| !is_safe_token(value, 120))
    {
        return Err(persisted_integrity("progress_phase is unsafe"));
    }
    if descriptor
        .progress_message
        .as_deref()
        .is_some_and(|value| !is_safe_message(value, 500))
    {
        return Err(persisted_integrity("progress_message is unsafe"));
    }
    if descriptor
        .error_code
        .as_deref()
        .is_some_and(|value| !is_safe_error_code(value))
    {
        return Err(persisted_integrity("error_code is unsafe"));
    }
    if descriptor
        .error_message
        .as_deref()
        .is_some_and(|value| !is_safe_message(value, 500))
    {
        return Err(persisted_integrity("error_message is unsafe"));
    }

    let has_complete_error = descriptor.error_code.is_some()
        && descriptor.error_message.is_some()
        && descriptor.error_retriable.is_some();
    let has_any_error = descriptor.error_code.is_some()
        || descriptor.error_message.is_some()
        || descriptor.error_retriable.is_some();
    if (status == JobStatus::Failed && !has_complete_error)
        || (status != JobStatus::Failed && has_any_error)
    {
        return Err(persisted_integrity(
            "status and persisted failure details disagree",
        ));
    }
    Ok(())
}

fn validate_persisted_status_fields(
    descriptor: &JobDescriptor,
    status: JobStatus,
) -> Result<(), JobError> {
    let started = descriptor.started_at.is_some();
    let finished = descriptor.finished_at.is_some();
    let valid_status_timestamps = match status {
        JobStatus::Queued => !started && !finished,
        JobStatus::Running => started && !finished,
        JobStatus::Succeeded => started && finished,
        JobStatus::Cancelling => !finished,
        JobStatus::Failed | JobStatus::Cancelled => finished,
    };
    if !valid_status_timestamps {
        return Err(persisted_integrity(
            "status and persisted lifecycle timestamps disagree",
        ));
    }
    if status == JobStatus::Queued
        && (descriptor.progress_phase.is_some() || descriptor.progress_message.is_some())
    {
        return Err(persisted_integrity(
            "queued job contains active progress details",
        ));
    }
    Ok(())
}

fn persisted_timestamp(value: &str, field: &str) -> Result<OffsetDateTime, JobError> {
    validate_timestamp(value)
        .map_err(|_| persisted_integrity(&format!("{field} is not a valid UTC RFC 3339 timestamp")))
}

fn persisted_integrity(detail: &str) -> JobError {
    JobError::DataIntegrity(detail.to_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    use rusqlite::{params, Connection, Result, TransactionBehavior};
    use sha2::{Digest, Sha256};
    use teratai_contracts::generated::job_enqueue_request::JobEnqueueRequest;
    use teratai_contracts::generated::job_failure_request::JobFailureRequest;
    use teratai_contracts::generated::job_progress_update_request::JobProgressUpdateRequest;
    use teratai_contracts::generated::job_transition_request::JobTransitionRequest;
    use teratai_contracts::generated::project_create_request::ProjectCreateRequest;
    use teratai_contracts::generated::project_manifest::ProjectManifest;
    use teratai_filesystem::REQUIRED_PROJECT_DIRECTORIES;

    use super::{
        event_id_at, full_project_validation_count, install_after_commit_barrier, snapshot_hash,
        transition_allowed, validate_persisted_descriptor, JobError, JobErrorKind, JobListCursor,
        JobStatus, JobStore, TransitionRequest, JOB_MIGRATION,
    };
    use crate::ProjectService;

    const PROJECT_MIGRATION: &str =
        include_str!("../../../migrations/metadata-sqlite/0001_project_core.sql");
    const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000100";
    const JOB_ID: &str = "00000000-0000-7000-8000-000000000210";
    const EVENT_ID: &str = "00000000-0000-7000-8000-000000000211";
    const CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000212";
    const RECOVERY_ID: &str = "00000000-0000-7000-8000-000000000213";
    const NOW: &str = "2026-07-20T12:00:00Z";
    const NOW_1: &str = "2026-07-20T12:01:00Z";
    const NOW_2: &str = "2026-07-20T12:02:00Z";
    const NOW_3: &str = "2026-07-20T12:03:00Z";
    const NOW_4: &str = "2026-07-20T12:04:00Z";
    const NOW_5: &str = "2026-07-20T12:05:00Z";
    const INVALID_CALENDAR_TIMESTAMPS: [&str; 4] = [
        "2026-13-10T12:00:00Z",
        "2026-01-32T12:00:00Z",
        "2026-01-10T23:60:00Z",
        "2026-01-10T23:59:60Z",
    ];

    #[test]
    fn terminal_state_is_immutable() {
        let path = create_schema_two_project("terminal");
        let store = JobStore::open(&path).expect("open store");
        store.enqueue_at(&enqueue(0), NOW).expect("enqueue job");

        let running = store
            .start_at(&transition(1), NOW_1)
            .expect("start queued job");
        let succeeded = store
            .succeed_at(&transition(running.revision), NOW_2)
            .expect("complete running job");
        assert!(matches!(
            store.fail_at(&failure(succeeded.revision), NOW_3),
            Err(JobError::InvalidTransition { .. })
        ));

        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn stale_revision_writes_nothing() {
        let path = create_schema_two_project("revision");
        let store = JobStore::open(&path).expect("open store");
        store.enqueue_at(&enqueue(0), NOW).expect("enqueue job");
        let before = job_event_audit_counts(&path);

        assert!(matches!(
            store.start_at(&transition(99), NOW_1),
            Err(JobError::RevisionConflict {
                expected: 99,
                actual: 1
            })
        ));
        assert_eq!(job_event_audit_counts(&path), before);

        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn independent_store_handles_observe_peer_writes_and_reach_revision_cas() {
        let path = create_schema_two_project("independent-handles");
        let first = JobStore::open(&path).expect("open first store");
        let second = JobStore::open(&path).expect("open second store before peer write");

        let queued = first.enqueue_at(&enqueue(0), NOW).expect("peer enqueue");
        assert_eq!(
            second.get(&queued.job_id).expect("read peer enqueue"),
            queued
        );
        let running = first
            .start_at(&transition(queued.revision), NOW_1)
            .expect("peer transition");
        assert_eq!(
            second.get(&running.job_id).expect("read peer transition"),
            running
        );
        assert!(matches!(
            second.start_at(&transition(queued.revision), NOW_2),
            Err(JobError::RevisionConflict {
                expected: 1,
                actual: 2
            })
        ));

        drop(second);
        drop(first);
        cleanup_project(&path);
    }

    #[test]
    fn simultaneous_peer_read_waits_for_authorized_identity_refresh() {
        let path = create_schema_two_project("simultaneous-peer-handles");
        let writer = JobStore::open(&path).expect("open writer store");
        let reader = JobStore::open(&path).expect("open reader store");
        let barrier = Arc::new(Barrier::new(2));
        let writer_barrier = Arc::clone(&barrier);
        let writer_thread = std::thread::spawn(move || {
            install_after_commit_barrier(Some(writer_barrier));
            let result = writer.enqueue_at(&enqueue(0), NOW);
            install_after_commit_barrier(None);
            result
        });
        barrier.wait();
        let (result_sender, result_receiver) = mpsc::channel();
        let reader_thread = std::thread::spawn(move || {
            result_sender
                .send(reader.get(&job_id(0)))
                .expect("send reader result");
        });
        let before_refresh = result_receiver.recv_timeout(Duration::from_millis(100));
        barrier.wait();

        let queued = writer_thread
            .join()
            .expect("writer thread")
            .expect("peer enqueue");
        let observed = match before_refresh {
            Err(RecvTimeoutError::Timeout) => result_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("reader completed after identity refresh"),
            Ok(result) => panic!("reader completed before identity refresh: {result:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("reader result channel disconnected"),
        }
        .expect("read after peer commit");
        reader_thread.join().expect("reader thread");
        assert_eq!(observed, queued);

        cleanup_project(&path);
    }

    #[test]
    fn persisted_descriptor_reads_reject_sql_valid_unsafe_text_and_cross_fields() {
        assert_persisted_tamper_rejected("multibyte-overflow", |connection| {
            let oversized = "é".repeat(251);
            connection
                .execute(
                    "UPDATE job SET progress_message = ?1 WHERE job_id = ?2",
                    params![oversized, job_id(0)],
                )
                .expect("inject byte-overflow message");
        });
        assert_persisted_tamper_rejected("control-character", |connection| {
            connection
                .execute(
                    "UPDATE job SET progress_message = 'baris pertama' || char(10) || 'baris kedua'
                     WHERE job_id = ?1",
                    [job_id(0)],
                )
                .expect("inject newline message");
        });
        let path = create_schema_two_project("sql-valid-control-character");
        let store = JobStore::open(&path).expect("open SQL-valid fixture");
        store.enqueue_at(&enqueue(0), NOW).expect("enqueue job");
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute(
                "UPDATE job SET progress_message = 'unsafe' || char(1) || 'message'
                 WHERE job_id = ?1",
                [job_id(0)],
            )
            .expect("inject SQL-valid control character");
        drop(connection);
        store
            .metadata
            .refresh_after_authorized_write()
            .expect("authorize SQL-valid fixture");
        assert!(matches!(
            store.get(&job_id(0)),
            Err(JobError::DataIntegrity(_))
        ));
        drop(store);
        cleanup_project(&path);
        assert_persisted_tamper_rejected("cross-field", |connection| {
            connection
                .execute(
                    "UPDATE job SET status = 'RUNNING', started_at = NULL WHERE job_id = ?1",
                    [job_id(0)],
                )
                .expect("inject invalid running timestamps");
        });
    }

    #[test]
    fn persisted_descriptor_validation_guards_mutation_and_recovery_reads() {
        let mutation_path = create_schema_two_project("invalid-before-mutation");
        let mutation_store = JobStore::open(&mutation_path).expect("open mutation fixture");
        mutation_store
            .enqueue_at(&enqueue(0), NOW)
            .expect("enqueue mutation fixture");
        let connection = Connection::open(mutation_path.join("metadata.sqlite"))
            .expect("open mutation metadata");
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .expect("enable migration-bypass fixture");
        connection
            .execute(
                "UPDATE job SET progress_message = 'unsafe' || char(9) || 'message'
                 WHERE job_id = ?1",
                [job_id(0)],
            )
            .expect("inject mutation control character");
        drop(connection);
        mutation_store
            .metadata
            .refresh_after_authorized_write()
            .expect("authorize migration-bypass fixture");
        assert!(matches!(
            mutation_store.start_at(&transition(1), NOW_1),
            Err(JobError::DataIntegrity(_))
        ));
        drop(mutation_store);
        cleanup_project(&mutation_path);

        let (recovery_path, recovery_store) = running_store("invalid-before-recovery");
        let connection = Connection::open(recovery_path.join("metadata.sqlite"))
            .expect("open recovery metadata");
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .expect("enable migration-bypass fixture");
        connection
            .execute(
                "UPDATE job SET progress_message = 'unsafe' || char(13) || 'message'
                 WHERE job_id = ?1",
                [job_id(0)],
            )
            .expect("inject recovery control character");
        drop(connection);
        recovery_store
            .metadata
            .refresh_after_authorized_write()
            .expect("authorize migration-bypass fixture");
        assert!(matches!(
            recovery_store.recover_interrupted_at(RECOVERY_ID, NOW_4),
            Err(JobError::DataIntegrity(_))
        ));
        drop(recovery_store);
        cleanup_project(&recovery_path);
    }

    #[test]
    fn persisted_descriptor_rejects_non_boolean_sqlite_integer() {
        let path = create_schema_two_project("invalid-persisted-boolean");
        let store = JobStore::open(&path).expect("open store");
        let descriptor = store.enqueue_at(&enqueue(0), NOW).expect("enqueue job");

        assert!(matches!(
            validate_persisted_descriptor(descriptor, Some(2)),
            Err(JobError::DataIntegrity(_))
        ));

        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn persisted_row_decode_rejects_boolean_two_from_migration_bypass() {
        let path = create_schema_two_project("boolean-two-bypass");
        let store = JobStore::open(&path).expect("open store");
        store.enqueue_at(&enqueue(0), NOW).expect("enqueue job");
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .expect("enable migration-bypass fixture");
        connection
            .execute(
                "UPDATE job
                 SET status = 'FAILED', finished_at = ?1, updated_at = ?1,
                     error_code = 'INTERRUPTED', error_message = 'Pekerjaan terhenti.',
                     error_retriable = 2
                 WHERE job_id = ?2",
                params![NOW_1, job_id(0)],
            )
            .expect("inject non-boolean integer");
        drop(connection);
        store
            .metadata
            .refresh_after_authorized_write()
            .expect("authorize migration-bypass fixture");

        assert!(matches!(
            store.get(&job_id(0)),
            Err(JobError::DataIntegrity(_))
        ));

        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn transition_matrix_is_exact_and_rejections_write_nothing() {
        let allowed = [
            (JobStatus::Queued, JobStatus::Running),
            (JobStatus::Queued, JobStatus::Cancelling),
            (JobStatus::Queued, JobStatus::Failed),
            (JobStatus::Running, JobStatus::Succeeded),
            (JobStatus::Running, JobStatus::Failed),
            (JobStatus::Running, JobStatus::Cancelling),
            (JobStatus::Cancelling, JobStatus::Cancelled),
            (JobStatus::Cancelling, JobStatus::Failed),
        ];

        for (from_index, from) in JobStatus::ALL.into_iter().enumerate() {
            for (to_index, to) in JobStatus::ALL.into_iter().enumerate() {
                let expected_allowed = allowed.contains(&(from, to));
                assert_eq!(
                    transition_allowed(from, to),
                    expected_allowed,
                    "unexpected policy for {from:?} -> {to:?}"
                );
                let label = format!("matrix-{from_index}-{to_index}");
                let (path, store) = seeded_store_for_status(&label, from);
                let current = store.get(&job_id(0)).expect("read seeded state");
                let request = transition(current.revision);
                let failure_request = failure(current.revision);
                let payload = if to == JobStatus::Failed {
                    TransitionRequest::Failure(&failure_request)
                } else {
                    TransitionRequest::Plain(&request)
                };
                let before = job_event_audit_counts(&path);
                let result = store.transition_to_at(payload, to, NOW_3);

                if expected_allowed {
                    assert_eq!(result.expect("allowed transition").status, to.as_str());
                    assert_eq!(
                        job_event_audit_counts(&path),
                        (before.0, before.1 + 1, before.2 + 1)
                    );
                } else {
                    assert!(matches!(result, Err(JobError::InvalidTransition { .. })));
                    assert_eq!(job_event_audit_counts(&path), before);
                    assert_eq!(store.get(&job_id(0)).expect("unchanged job"), current);
                }
                drop(store);
                cleanup_project(&path);
            }
        }
    }

    #[test]
    fn progress_rejects_regression_but_allows_phase_reset() {
        let (path, store) = running_store("progress");
        let first = store
            .update_progress_at(&progress(2, 40, Some(100), "scan"), NOW_2)
            .expect("record scan progress");
        let before_rejection = job_event_audit_counts(&path);

        assert!(matches!(
            store.update_progress_at(&progress(first.revision, 39, Some(100), "scan"), NOW_3),
            Err(JobError::InvalidRequest(_))
        ));
        assert_eq!(job_event_audit_counts(&path), before_rejection);
        assert_eq!(
            store
                .update_progress_at(&progress(first.revision, 0, Some(10), "write"), NOW_3)
                .expect("reset progress for a new phase")
                .progress_current,
            0
        );

        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn progress_rejects_invalid_bounds_and_safe_fields_without_writes() {
        let (path, store) = running_store("progress-validation");
        let invalid = [
            progress(2, -1, Some(100), "scan"),
            progress(2, 1, Some(0), "scan"),
            progress(2, 101, Some(100), "scan"),
            JobProgressUpdateRequest {
                phase: "Invalid Phase".to_owned(),
                ..progress(2, 1, Some(100), "scan")
            },
            JobProgressUpdateRequest {
                message: " progress".to_owned(),
                ..progress(2, 1, Some(100), "scan")
            },
            JobProgressUpdateRequest {
                unit: Some("bad unit".to_owned()),
                ..progress(2, 1, Some(100), "scan")
            },
        ];

        for request in invalid {
            let before = job_event_audit_counts(&path);
            assert!(matches!(
                store.update_progress_at(&request, NOW_2),
                Err(JobError::InvalidRequest(_))
            ));
            assert_eq!(job_event_audit_counts(&path), before);
        }
        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn cancellation_request_is_idempotent_and_does_not_finish_early() {
        let (path, store) = running_store("cancel");
        let running = store.get(&job_id(0)).expect("read running job");
        let cancelling = store
            .request_cancellation_at(&transition(running.revision), NOW_2)
            .expect("request cancellation");
        assert_eq!(cancelling.status, "CANCELLING");
        assert!(cancelling.finished_at.is_none());
        let counts = job_event_audit_counts(&path);

        assert_eq!(
            store
                .request_cancellation_at(&transition(cancelling.revision), NOW_3)
                .expect("repeat cancellation request"),
            cancelling
        );
        assert_eq!(job_event_audit_counts(&path), counts);
        let cancelled = store
            .complete_cancellation_at(&transition(cancelling.revision), NOW_3)
            .expect("complete cooperative cancellation");
        assert_eq!(cancelled.status, "CANCELLED");
        assert_eq!(cancelled.finished_at.as_deref(), Some(NOW_3));

        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn recovery_marks_active_jobs_interrupted_once() {
        let (path, store) = store_with_running_cancelling_and_queued_jobs("recovery");

        let recovered = store
            .recover_interrupted_at(RECOVERY_ID, NOW_4)
            .expect("recover interrupted jobs");
        assert_eq!(recovered.len(), 2);
        assert!(recovered.iter().all(|job| {
            job.status == "FAILED"
                && job.error_code.as_deref() == Some("INTERRUPTED")
                && job.error_retriable == Some(true)
                && job.finished_at.as_deref() == Some(NOW_4)
        }));
        assert_eq!(store.get(&job_id(2)).expect("queued job").status, "QUEUED");
        let counts = job_event_audit_counts(&path);

        assert!(store
            .recover_interrupted_at(RECOVERY_ID, NOW_5)
            .expect("repeat recovery")
            .is_empty());
        assert_eq!(job_event_audit_counts(&path), counts);

        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn audit_failure_rolls_back_snapshot_and_job_event() {
        let path = create_schema_two_project("atomicity");
        let store = JobStore::open(&path).expect("open store");
        store.enqueue_at(&enqueue(0), NOW).expect("enqueue job");
        drop(store);
        install_audit_abort_trigger(&path, "job.started");
        let store = JobStore::open(&path).expect("reopen store");
        let before = job_event_audit_counts(&path);

        assert!(matches!(
            store.start_at(&transition(1), NOW_1),
            Err(JobError::Database(_))
        ));
        assert_eq!(job_event_audit_counts(&path), before);
        assert_eq!(
            store.get(&job_id(0)).expect("rolled-back job").status,
            "QUEUED"
        );

        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn jobs_survive_reopen_and_list_is_bounded() {
        let path = create_schema_two_project("list");
        let store = JobStore::open(&path).expect("open store");
        for index in 0..3 {
            store
                .enqueue_at(&enqueue(index), &timestamp(index))
                .expect("enqueue job");
        }
        drop(store);

        let reopened = JobStore::open(&path).expect("reopen store");
        let first = reopened.list(2, None).expect("first page");
        assert_eq!(
            first
                .items
                .iter()
                .map(|item| item.job_id.clone())
                .collect::<Vec<_>>(),
            vec![job_id(2), job_id(1)]
        );
        assert_eq!(
            first.next_cursor,
            Some(JobListCursor {
                updated_at: timestamp(1),
                job_id: job_id(1),
            })
        );

        let second = reopened.list(2, first.next_cursor).expect("second page");
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0].job_id, job_id(0));
        assert!(second.next_cursor.is_none());
        drop(reopened);
        cleanup_project(&path);
    }

    #[test]
    fn list_breaks_timestamp_ties_by_descending_job_id_without_duplicates() {
        let path = create_schema_two_project("list-ties");
        let store = JobStore::open(&path).expect("open store");
        for index in 0..3 {
            store.enqueue_at(&enqueue(index), NOW).expect("enqueue job");
        }

        let first = store.list(1, None).expect("first page");
        let second = store.list(1, first.next_cursor).expect("second page");
        let third = store.list(1, second.next_cursor).expect("third page");
        assert_eq!(first.items[0].job_id, job_id(2));
        assert_eq!(second.items[0].job_id, job_id(1));
        assert_eq!(third.items[0].job_id, job_id(0));
        assert!(third.next_cursor.is_none());
        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn store_rejects_schema_one_without_mutation() {
        let path = create_schema_one_project("job-v1");
        let before = control_file_bytes(&path);
        let entries_before = project_entry_names(&path);

        assert!(matches!(
            JobStore::open(&path),
            Err(JobError::IncompatibleSchema {
                expected: 2,
                actual: 1
            })
        ));
        assert_eq!(control_file_bytes(&path), before);
        assert_eq!(project_entry_names(&path), entries_before);
        cleanup_project(&path);
    }

    #[test]
    fn store_rejects_manifest_database_mismatch_and_tampered_identity() {
        let manifest_mismatch = create_schema_two_project("manifest-mismatch");
        let manifest_path = manifest_mismatch.join("manifest.json");
        let mut manifest: ProjectManifest =
            serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
                .expect("decode manifest");
        manifest.metadata_schema_version = 1;
        let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("encode manifest");
        manifest_bytes.push(b'\n');
        fs::write(&manifest_path, manifest_bytes).expect("write mismatched manifest");

        let tampered_identity = create_schema_two_project("tampered-identity");
        let connection =
            Connection::open(tampered_identity.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute("UPDATE project SET name = 'Tampered Project'", [])
            .expect("tamper project identity");
        drop(connection);

        let mismatch_result = JobStore::open(&manifest_mismatch);
        let tampered_result = JobStore::open(&tampered_identity);
        cleanup_project(&manifest_mismatch);
        cleanup_project(&tampered_identity);
        assert!(matches!(mismatch_result, Err(JobError::DataIntegrity(_))));
        assert!(matches!(tampered_result, Err(JobError::DataIntegrity(_))));
    }

    #[test]
    fn store_rejects_malformed_schema_two_job_objects() {
        let path = create_schema_two_project("malformed-schema-two");
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute("DROP TABLE job_event", [])
            .expect("remove required job history table");
        drop(connection);

        let result = JobStore::open(&path);
        cleanup_project(&path);
        assert!(matches!(result, Err(JobError::DataIntegrity(_))));
    }

    #[test]
    fn store_rejects_extra_migration_and_broken_audit_authorization() {
        let extra_migration = create_schema_two_project("extra-migration");
        let connection =
            Connection::open(extra_migration.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute(
                "INSERT INTO schema_migrations (version, name, applied_at)
                 VALUES (3, 'unexpected', ?1)",
                [NOW],
            )
            .expect("insert unexpected migration");
        drop(connection);

        let broken_audit = create_schema_two_project("broken-audit");
        let connection =
            Connection::open(broken_audit.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute_batch("DROP TRIGGER audit_event_prevent_update")
            .expect("drop audit update guard for tamper fixture");
        connection
            .execute(
                "UPDATE audit_event SET after_hash = 'sha256:tampered'
                 WHERE action = 'project.created'",
                [],
            )
            .expect("break audit authorization chain");
        drop(connection);

        let migration_result = JobStore::open(&extra_migration);
        let audit_result = JobStore::open(&broken_audit);
        cleanup_project(&extra_migration);
        cleanup_project(&broken_audit);
        assert!(matches!(migration_result, Err(JobError::DataIntegrity(_))));
        assert!(matches!(audit_result, Err(JobError::DataIntegrity(_))));
    }

    #[test]
    fn operations_revalidate_project_after_metadata_replacement() {
        let path = create_schema_two_project("replacement-target");
        let replacement = create_schema_two_project("replacement-source");
        let replacement_store = JobStore::open(&replacement).expect("open replacement store");
        replacement_store
            .enqueue_at(&enqueue(0), NOW)
            .expect("seed replacement job");
        drop(replacement_store);
        let store = JobStore::open(&path).expect("open original store");
        fs::copy(
            replacement.join("metadata.sqlite"),
            path.join("metadata.sqlite"),
        )
        .expect("replace metadata after store open");

        let enqueue_result = store.enqueue_at(&enqueue(9), NOW);
        let get_result = store.get(&job_id(0));
        let list_result = store.list(10, None);
        drop(store);
        cleanup_project(&path);
        cleanup_project(&replacement);
        assert!(matches!(enqueue_result, Err(JobError::DataIntegrity(_))));
        assert!(matches!(get_result, Err(JobError::DataIntegrity(_))));
        assert!(matches!(list_result, Err(JobError::DataIntegrity(_))));
    }

    #[test]
    fn operations_reject_byte_identical_metadata_replacement() {
        let path = create_schema_two_project("byte-identical-replacement");
        let store = JobStore::open(&path).expect("open store");
        let metadata_path = path.join("metadata.sqlite");
        let original = fs::read(&metadata_path).expect("read original metadata");
        fs::write(&metadata_path, original).expect("replace metadata with identical bytes");

        let enqueue_result = store.enqueue_at(&enqueue(0), NOW);
        let get_result = store.get(&job_id(0));
        let list_result = store.list(10, None);
        drop(store);
        cleanup_project(&path);
        assert!(matches!(enqueue_result, Err(JobError::DataIntegrity(_))));
        assert!(matches!(get_result, Err(JobError::DataIntegrity(_))));
        assert!(matches!(list_result, Err(JobError::DataIntegrity(_))));
    }

    #[test]
    fn operations_reject_valid_same_project_replacement() {
        let path = create_schema_two_project("same-project-target");
        let replacement = create_schema_two_project("same-project-source");
        let replacement_store = JobStore::open(&replacement).expect("open replacement store");
        replacement_store
            .enqueue_at(&enqueue(0), NOW)
            .expect("seed replacement job");
        drop(replacement_store);
        let store = JobStore::open(&path).expect("open original store");
        fs::copy(
            replacement.join("manifest.json"),
            path.join("manifest.json"),
        )
        .expect("replace manifest");
        fs::copy(
            replacement.join("metadata.sqlite"),
            path.join("metadata.sqlite"),
        )
        .expect("replace metadata with valid same-project database");

        let enqueue_result = store.enqueue_at(&enqueue(9), NOW);
        let get_result = store.get(&job_id(0));
        let list_result = store.list(10, None);
        drop(store);
        cleanup_project(&path);
        cleanup_project(&replacement);
        assert!(matches!(enqueue_result, Err(JobError::DataIntegrity(_))));
        assert!(matches!(get_result, Err(JobError::DataIntegrity(_))));
        assert!(matches!(list_result, Err(JobError::DataIntegrity(_))));
    }

    #[test]
    fn repeated_list_does_not_repeat_full_project_integrity_validation() {
        let path = create_schema_two_project("lightweight-list");
        let store = JobStore::open(&path).expect("open store");
        let validations_after_open = full_project_validation_count();

        for _ in 0..3 {
            assert!(store.list(10, None).expect("list jobs").items.is_empty());
        }
        assert_eq!(full_project_validation_count(), validations_after_open);
        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn operations_reject_metadata_symlink_created_after_store_open() {
        let path = create_schema_two_project("metadata-symlink");
        let store = JobStore::open(&path).expect("open store");
        let metadata_path = path.join("metadata.sqlite");
        let original_path = path.join("metadata.original.sqlite");
        if let Err(error) = fs::rename(&metadata_path, &original_path) {
            drop(store);
            cleanup_project(&path);
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(32)
            {
                return;
            }
            panic!("move metadata aside: {error}");
        }
        if let Err(error) = symlink_file(&original_path, &metadata_path) {
            fs::rename(&original_path, &metadata_path).expect("restore metadata after skip");
            drop(store);
            cleanup_project(&path);
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("create metadata symlink: {error}");
        }

        let enqueue_result = store.enqueue_at(&enqueue(0), NOW);
        let get_result = store.get(&job_id(0));
        let list_result = store.list(10, None);
        drop(store);
        cleanup_project(&path);
        assert!(matches!(enqueue_result, Err(JobError::DataIntegrity(_))));
        assert!(matches!(get_result, Err(JobError::DataIntegrity(_))));
        assert!(matches!(list_result, Err(JobError::DataIntegrity(_))));
    }

    #[test]
    fn enqueue_validates_exact_flat_fields_before_writing() {
        let path = create_schema_two_project("validation");
        let store = JobStore::open(&path).expect("open store");
        let invalid_requests = [
            JobEnqueueRequest {
                job_id: "not-a-uuid".to_owned(),
                ..enqueue(0)
            },
            JobEnqueueRequest {
                correlation_id: "00000000-0000-6000-8000-000000000301".to_owned(),
                ..enqueue(0)
            },
            JobEnqueueRequest {
                kind: "Invalid.Kind".to_owned(),
                ..enqueue(0)
            },
            JobEnqueueRequest {
                kind: format!("a{}", "b".repeat(120)),
                ..enqueue(0)
            },
            JobEnqueueRequest {
                progress_total: Some(0),
                ..enqueue(0)
            },
            JobEnqueueRequest {
                progress_unit: Some(" step".to_owned()),
                ..enqueue(0)
            },
            JobEnqueueRequest {
                progress_unit: Some("x".repeat(33)),
                ..enqueue(0)
            },
        ];

        for request in invalid_requests {
            assert!(matches!(
                store.enqueue_at(&request, NOW),
                Err(JobError::InvalidRequest(_))
            ));
        }
        assert_eq!(job_event_audit_counts(&path), (0, 0, 1));
        assert!(matches!(
            store.list(0, None),
            Err(JobError::InvalidRequest(_))
        ));
        assert!(matches!(
            store.list(101, None),
            Err(JobError::InvalidRequest(_))
        ));
        assert!(matches!(
            store.list(
                1,
                Some(JobListCursor {
                    updated_at: "not-a-timestamp".to_owned(),
                    job_id: job_id(0),
                })
            ),
            Err(JobError::InvalidRequest(_))
        ));
        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn enqueue_persists_snapshot_event_and_audit_atomically() {
        let path = create_schema_two_project("atomic-enqueue");
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_job_enqueue_audit
                 BEFORE INSERT ON audit_event
                 WHEN NEW.action = 'job.queued'
                 BEGIN SELECT RAISE(ABORT, 'test audit failure'); END;",
            )
            .expect("install abort trigger");
        drop(connection);
        let store = JobStore::open(&path).expect("open store");

        assert!(matches!(
            store.enqueue_at(&enqueue(0), NOW),
            Err(JobError::Database(_))
        ));
        assert_eq!(job_event_audit_counts(&path), (0, 0, 1));
        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn enqueue_event_ids_are_distinct_lowercase_uuid_v7_values() {
        let path = create_schema_two_project("event-ids");
        let store = JobStore::open(&path).expect("open store");
        store
            .enqueue_at(&enqueue(0), NOW)
            .expect("enqueue job with events");
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        let job_event_id: String = connection
            .query_row("SELECT event_id FROM job_event", [], |row| row.get(0))
            .expect("read job event ID");
        let audit_event_id: String = connection
            .query_row(
                "SELECT event_id FROM audit_event WHERE action = 'job.queued'",
                [],
                |row| row.get(0),
            )
            .expect("read audit event ID");
        assert!(is_lowercase_uuid_v7(&job_event_id));
        assert!(is_lowercase_uuid_v7(&audit_event_id));
        assert!(job_event_id.starts_with("019f7f65-b600-7"));
        assert!(audit_event_id.starts_with("019f7f65-b600-7"));
        assert_ne!(job_event_id, audit_event_id);
        assert_ne!(
            event_id_at("job-event", &job_id(0), 1, "job.queued", NOW).expect("revision one ID"),
            event_id_at("job-event", &job_id(0), 2, "job.queued", NOW).expect("revision two ID")
        );
        drop(connection);
        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn enqueue_audit_hash_matches_the_persisted_snapshot() {
        let path = create_schema_two_project("audit-hash");
        let store = JobStore::open(&path).expect("open store");
        let enqueued = store
            .enqueue_at(&enqueue(0), NOW)
            .expect("enqueue job with audit");
        let persisted = store.get(&enqueued.job_id).expect("read persisted job");
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        let (before_hash, after_hash): (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT before_hash, after_hash FROM audit_event WHERE action = 'job.queued'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read audit hashes");

        assert!(before_hash.is_none());
        assert_eq!(
            after_hash.as_deref(),
            Some(snapshot_hash(&persisted).expect("hash snapshot").as_str())
        );
        drop(connection);
        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn get_rejects_invalid_id_and_reports_missing_job_safely() {
        let path = create_schema_two_project("get-errors");
        let store = JobStore::open(&path).expect("open store");

        assert!(matches!(
            store.get("not-a-uuid"),
            Err(JobError::InvalidRequest(_))
        ));
        assert!(matches!(
            store.get(&job_id(9)),
            Err(JobError::JobNotFound(_))
        ));
        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn every_store_connection_enables_foreign_keys() {
        let path = create_schema_two_project("foreign-keys");
        let store = JobStore::open(&path).expect("open store");
        let operation = store
            .metadata
            .begin_operation()
            .expect("begin pinned operation");
        let connection = store
            .read_connection(&operation)
            .expect("open store connection");
        let foreign_keys: i64 = connection
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .expect("read foreign key setting");

        assert_eq!(foreign_keys, 1);
        assert!(connection.is_readonly("main").expect("read access mode"));
        drop(connection);
        drop(operation);
        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn schema_two_enforces_job_shape_and_append_only_events() {
        let connection = migrated_memory_database();

        assert_eq!(user_version(&connection), 2);
        assert!(insert_job_with_status(&connection, "UNKNOWN").is_err());
        insert_job_with_status(&connection, "QUEUED").expect("insert valid queued job");

        assert!(connection
            .execute("UPDATE job SET revision = 0 WHERE job_id = ?1", [JOB_ID])
            .is_err());
        assert!(connection
            .execute(
                "UPDATE job SET updated_at = '2026-07-20 12:00:00' WHERE job_id = ?1",
                [JOB_ID],
            )
            .is_err());
        assert!(connection
            .execute(
                "UPDATE job SET progress_current = 101, progress_total = 100 WHERE job_id = ?1",
                [JOB_ID],
            )
            .is_err());
        assert!(connection
            .execute(
                "UPDATE job SET status = 'SUCCEEDED' WHERE job_id = ?1",
                [JOB_ID],
            )
            .is_err());
        assert!(connection
            .execute(
                "UPDATE job SET status = 'FAILED', finished_at = ?2 WHERE job_id = ?1",
                params![JOB_ID, NOW],
            )
            .is_err());

        insert_job_event(&connection, "job.queued", "QUEUED").expect("insert valid queued event");
        assert!(connection.execute("DELETE FROM job_event", []).is_err());
        assert!(connection
            .execute("UPDATE job_event SET event_type = 'changed'", [])
            .is_err());
    }

    #[test]
    fn schema_two_rejects_invalid_rfc3339_timestamp() {
        let connection = migrated_memory_database();

        for invalid_timestamp in ["xxxx-99-99T99:99:99Z", "2026-07-20T24:00:00Z"] {
            let result = connection.execute(
                "INSERT INTO job (job_id, project_id, kind, status, correlation_id, revision, created_at, updated_at, progress_current) VALUES (?1, ?2, 'system.mock_long', 'QUEUED', ?3, 1, ?4, ?5, 0)",
                params![JOB_ID, PROJECT_ID, CORRELATION_ID, invalid_timestamp, NOW],
            );

            assert!(result.is_err(), "accepted {invalid_timestamp}");
        }
    }

    #[test]
    fn schema_two_rejects_invalid_job_calendar_timestamp() {
        let connection = migrated_memory_database();

        for invalid_timestamp in INVALID_CALENDAR_TIMESTAMPS {
            let result = connection.execute(
                "INSERT INTO job (job_id, project_id, kind, status, correlation_id, revision, created_at, updated_at, progress_current) VALUES (?1, ?2, 'system.mock_long', 'QUEUED', ?3, 1, ?4, ?5, 0)",
                params![JOB_ID, PROJECT_ID, CORRELATION_ID, invalid_timestamp, NOW],
            );

            assert!(
                result.is_err(),
                "accepted job timestamp {invalid_timestamp}"
            );
        }
    }

    #[test]
    fn schema_two_rejects_invalid_job_event_calendar_timestamp() {
        let connection = migrated_memory_database();
        insert_job_with_status(&connection, "QUEUED").expect("insert queued job");

        for invalid_timestamp in INVALID_CALENDAR_TIMESTAMPS {
            let result =
                insert_job_event_at(&connection, "job.queued", "QUEUED", invalid_timestamp);

            assert!(
                result.is_err(),
                "accepted job event timestamp {invalid_timestamp}"
            );
        }
    }

    #[test]
    fn schema_two_rejects_started_timestamp_for_queued_job() {
        let connection = migrated_memory_database();

        let result = connection.execute(
            "INSERT INTO job (job_id, project_id, kind, status, correlation_id, revision, created_at, started_at, updated_at, progress_current) VALUES (?1, ?2, 'system.mock_long', 'QUEUED', ?3, 1, ?4, ?4, ?4, 0)",
            params![JOB_ID, PROJECT_ID, CORRELATION_ID, NOW],
        );

        assert!(result.is_err());
    }

    #[test]
    fn schema_two_rejects_unsafe_text_by_bytes_and_common_controls() {
        let connection = migrated_memory_database();
        insert_job_with_status(&connection, "QUEUED").expect("insert queued job");

        let oversized = "é".repeat(251);
        assert!(connection
            .execute(
                "UPDATE job SET progress_message = ?1 WHERE job_id = ?2",
                params![oversized, JOB_ID],
            )
            .is_err());
        assert!(connection
            .execute(
                "UPDATE job SET progress_message = 'line one' || char(10) || 'line two'
                 WHERE job_id = ?1",
                [JOB_ID],
            )
            .is_err());
        assert!(connection
            .execute(
                "UPDATE job SET progress_unit = 'bad unit' WHERE job_id = ?1",
                [JOB_ID],
            )
            .is_err());
    }

    fn assert_persisted_tamper_rejected(label: &str, tamper: impl FnOnce(&Connection)) {
        let path = create_schema_two_project(label);
        let store = JobStore::open(&path).expect("open store");
        store.enqueue_at(&enqueue(0), NOW).expect("enqueue job");
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .expect("enable migration-bypass fixture");
        tamper(&connection);
        drop(connection);
        store
            .metadata
            .refresh_after_authorized_write()
            .expect("authorize migration-bypass fixture");
        assert!(matches!(
            store.get(&job_id(0)),
            Err(JobError::DataIntegrity(_))
        ));
        assert!(matches!(
            store.list(10, None),
            Err(JobError::DataIntegrity(_))
        ));
        drop(store);
        cleanup_project(&path);
    }

    #[test]
    fn classifies_job_errors_without_rendering_raw_database_details() {
        let database = JobError::Database(rusqlite::Error::InvalidQuery);
        let conflict = JobError::RevisionConflict {
            expected: 3,
            actual: 4,
        };

        assert_eq!(database.kind(), JobErrorKind::Database);
        assert_eq!(database.to_string(), "job metadata operation failed");
        assert_eq!(conflict.kind(), JobErrorKind::RevisionConflict);
    }

    #[test]
    fn job_error_display_never_reflects_arbitrary_input() {
        const SENSITIVE: &str = "D:\\private\\project.teratai raw sqlite failure";
        let errors = [
            JobError::InvalidRequest(SENSITIVE.to_owned()),
            JobError::JobNotFound(SENSITIVE.to_owned()),
            JobError::InvalidTransition {
                from: SENSITIVE.to_owned(),
                to: SENSITIVE.to_owned(),
            },
            JobError::RevisionConflict {
                expected: 1,
                actual: 2,
            },
            JobError::IncompatibleSchema {
                expected: 2,
                actual: 3,
            },
            JobError::DataIntegrity(SENSITIVE.to_owned()),
            JobError::Database(rusqlite::Error::InvalidParameterName(SENSITIVE.to_owned())),
            JobError::Timestamp(SENSITIVE.to_owned()),
        ];

        for error in errors {
            let rendered = error.to_string();
            assert!(
                !rendered.contains(SENSITIVE),
                "{} reflected arbitrary input",
                error.kind() as u8
            );
        }
    }

    fn create_schema_two_project(label: &str) -> PathBuf {
        let path = fixture_target(label);
        let request = ProjectCreateRequest {
            name: format!("Job {label}"),
            project_id: PROJECT_ID.to_owned(),
            project_path: path.to_string_lossy().into_owned(),
            request_id: "00000000-0000-7000-8000-000000000101".to_owned(),
        };
        ProjectService::create_at(&request, NOW).expect("create schema-two project");
        path
    }

    fn create_schema_one_project(label: &str) -> PathBuf {
        let path = fixture_target(label);
        fs::create_dir(&path).expect("create schema-one project");
        for directory in REQUIRED_PROJECT_DIRECTORIES {
            fs::create_dir_all(path.join(directory)).expect("create project directory");
        }
        let manifest = ProjectManifest {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            created_at: NOW.to_owned(),
            metadata_schema_version: 1,
            name: format!("Job {label}"),
            project_id: PROJECT_ID.to_owned(),
            schema_version: "1.0.0".to_owned(),
        };
        let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("encode manifest");
        manifest_bytes.push(b'\n');
        fs::write(path.join("manifest.json"), &manifest_bytes).expect("write manifest");
        let manifest_hash = format!("sha256:{:x}", Sha256::digest(&manifest_bytes));
        let mut connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .pragma_update(None, "foreign_keys", true)
            .expect("enable foreign keys");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("start schema-one transaction");
        transaction
            .execute_batch(PROJECT_MIGRATION)
            .expect("apply schema one");
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (1, 'project_core', ?1)",
                [NOW],
            )
            .expect("record schema one");
        transaction
            .execute(
                "INSERT INTO project (singleton, project_id, name, created_at, app_version, manifest_schema_version)
                 VALUES (1, ?1, ?2, ?3, ?4, '1.0.0')",
                params![PROJECT_ID, manifest.name, NOW, manifest.app_version],
            )
            .expect("seed project");
        transaction
            .execute(
                "INSERT INTO audit_event (
                    event_id, actor, action, target_type, target_id,
                    before_hash, after_hash, occurred_at, correlation_id
                 ) VALUES (?1, 'local-user', 'project.created', 'project', ?2, NULL, ?3, ?4, ?1)",
                params![
                    "00000000-0000-7000-8000-000000000101",
                    PROJECT_ID,
                    manifest_hash,
                    NOW,
                ],
            )
            .expect("seed project audit");
        transaction.commit().expect("commit schema one");
        drop(connection);
        path
    }

    fn seeded_store_for_status(label: &str, status: JobStatus) -> (PathBuf, JobStore) {
        let path = create_schema_two_project(label);
        let store = JobStore::open(&path).expect("open store for enqueue");
        store.enqueue_at(&enqueue(0), NOW).expect("enqueue job");
        drop(store);

        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        let (revision, started_at, finished_at, error_code, error_message, error_retriable) =
            match status {
                JobStatus::Queued => (1, None, None, None, None, None),
                JobStatus::Running => (2, Some(NOW_1), None, None, None, None),
                JobStatus::Succeeded => (3, Some(NOW_1), Some(NOW_2), None, None, None),
                JobStatus::Failed => (
                    2,
                    None,
                    Some(NOW_1),
                    Some("OPERATION_FAILED"),
                    Some("Pekerjaan tidak dapat diselesaikan."),
                    Some(0),
                ),
                JobStatus::Cancelling => (2, None, None, None, None, None),
                JobStatus::Cancelled => (3, None, Some(NOW_2), None, None, None),
            };
        connection
            .execute(
                "UPDATE job
                 SET status = ?1, revision = ?2, started_at = ?3, finished_at = ?4,
                     updated_at = ?5, error_code = ?6, error_message = ?7,
                     error_retriable = ?8
                 WHERE job_id = ?9",
                params![
                    status.as_str(),
                    revision,
                    started_at,
                    finished_at,
                    if revision == 1 { NOW } else { NOW_2 },
                    error_code,
                    error_message,
                    error_retriable,
                    job_id(0),
                ],
            )
            .expect("seed job status");
        drop(connection);
        let store = JobStore::open(&path).expect("reopen seeded store");
        (path, store)
    }

    fn fixture_target(label: &str) -> PathBuf {
        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
        let fixture = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "teratai-job-{label}-{}-{fixture}",
            std::process::id()
        ));
        fs::create_dir(&parent).expect("create fixture parent");
        parent.join(format!("{label}.teratai"))
    }

    fn enqueue(index: u8) -> JobEnqueueRequest {
        JobEnqueueRequest {
            correlation_id: format!("00000000-0000-7000-8000-0000000003{index:02}"),
            job_id: job_id(index),
            kind: "system.mock_long".to_owned(),
            progress_total: Some(100),
            progress_unit: Some("step".to_owned()),
        }
    }

    fn transition(expected_revision: i64) -> JobTransitionRequest {
        transition_for(&job_id(0), expected_revision)
    }

    fn transition_for(job_id: &str, expected_revision: i64) -> JobTransitionRequest {
        JobTransitionRequest {
            correlation_id: CORRELATION_ID.to_owned(),
            expected_revision,
            job_id: job_id.to_owned(),
        }
    }

    fn failure(expected_revision: i64) -> JobFailureRequest {
        JobFailureRequest {
            correlation_id: CORRELATION_ID.to_owned(),
            error_code: "OPERATION_FAILED".to_owned(),
            error_message: "Pekerjaan tidak dapat diselesaikan.".to_owned(),
            error_retriable: false,
            expected_revision,
            job_id: job_id(0),
        }
    }

    fn progress(
        expected_revision: i64,
        current: i64,
        total: Option<i64>,
        phase: &str,
    ) -> JobProgressUpdateRequest {
        JobProgressUpdateRequest {
            correlation_id: CORRELATION_ID.to_owned(),
            current,
            expected_revision,
            job_id: job_id(0),
            message: "Memproses pekerjaan.".to_owned(),
            phase: phase.to_owned(),
            total,
            unit: Some("row".to_owned()),
        }
    }

    fn running_store(label: &str) -> (PathBuf, JobStore) {
        let path = create_schema_two_project(label);
        let store = JobStore::open(&path).expect("open store");
        store.enqueue_at(&enqueue(0), NOW).expect("enqueue job");
        store
            .start_at(&transition(1), NOW_1)
            .expect("start queued job");
        (path, store)
    }

    fn store_with_running_cancelling_and_queued_jobs(label: &str) -> (PathBuf, JobStore) {
        let path = create_schema_two_project(label);
        let store = JobStore::open(&path).expect("open store");
        for index in 0..3 {
            store
                .enqueue_at(&enqueue(index), NOW)
                .expect("enqueue recovery fixture job");
        }
        store
            .start_at(&transition_for(&job_id(0), 1), NOW_1)
            .expect("start running recovery job");
        let running = store
            .start_at(&transition_for(&job_id(1), 1), NOW_1)
            .expect("start cancelling recovery job");
        store
            .request_cancellation_at(&transition_for(&job_id(1), running.revision), NOW_2)
            .expect("request cancellation for recovery job");
        (path, store)
    }

    fn install_audit_abort_trigger(path: &Path, action: &str) {
        assert!(super::is_safe_token(action, 120));
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute_batch(&format!(
                "CREATE TRIGGER fail_job_audit
                 BEFORE INSERT ON audit_event
                 WHEN NEW.action = '{action}'
                 BEGIN SELECT RAISE(ABORT, 'test audit failure'); END;"
            ))
            .expect("install audit abort trigger");
    }

    fn job_id(index: u8) -> String {
        format!("00000000-0000-7000-8000-0000000002{index:02}")
    }

    fn timestamp(index: u8) -> String {
        format!("2026-07-20T12:00:{index:02}Z")
    }

    fn cleanup_project(path: &Path) {
        fs::remove_dir_all(path.parent().expect("fixture parent")).expect("cleanup fixture");
    }

    fn control_file_bytes(path: &Path) -> (Vec<u8>, Vec<u8>) {
        (
            fs::read(path.join("manifest.json")).expect("read manifest"),
            fs::read(path.join("metadata.sqlite")).expect("read metadata"),
        )
    }

    fn project_entry_names(path: &Path) -> Vec<String> {
        let mut names = fs::read_dir(path)
            .expect("read project directory")
            .map(|entry| {
                entry
                    .expect("read project entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    #[cfg(unix)]
    fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(original, link)
    }

    #[cfg(windows)]
    fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(original, link)
    }

    fn job_event_audit_counts(path: &Path) -> (i64, i64, i64) {
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        (
            connection
                .query_row("SELECT COUNT(*) FROM job", [], |row| row.get(0))
                .expect("job count"),
            connection
                .query_row("SELECT COUNT(*) FROM job_event", [], |row| row.get(0))
                .expect("job event count"),
            connection
                .query_row("SELECT COUNT(*) FROM audit_event", [], |row| row.get(0))
                .expect("audit event count"),
        )
    }

    fn is_lowercase_uuid_v7(value: &str) -> bool {
        let bytes = value.as_bytes();
        bytes.len() == 36
            && bytes[8] == b'-'
            && bytes[13] == b'-'
            && bytes[14] == b'7'
            && bytes[18] == b'-'
            && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
            && bytes[23] == b'-'
            && bytes.iter().enumerate().all(|(index, byte)| {
                matches!(index, 8 | 13 | 18 | 23)
                    || byte.is_ascii_digit()
                    || (b'a'..=b'f').contains(byte)
            })
    }

    fn migrated_memory_database() -> Connection {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        connection
            .pragma_update(None, "foreign_keys", true)
            .expect("enable foreign keys");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("start migration transaction");
        transaction
            .execute_batch(PROJECT_MIGRATION)
            .expect("apply schema one");
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (1, 'project_core', ?1)",
                [NOW],
            )
            .expect("record schema one");
        transaction
            .execute(
                "INSERT INTO project (singleton, project_id, name, created_at, app_version, manifest_schema_version) VALUES (1, ?1, 'Test Project', ?2, '0.1.0', '1.0.0')",
                params![PROJECT_ID, NOW],
            )
            .expect("seed project");
        transaction
            .execute_batch(JOB_MIGRATION)
            .expect("apply schema two");
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (2, 'job_runtime', ?1)",
                [NOW],
            )
            .expect("record schema two");
        transaction.commit().expect("commit migrations");
        connection
    }

    fn user_version(connection: &Connection) -> i64 {
        connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read user_version")
    }

    fn insert_job_with_status(connection: &Connection, status: &str) -> Result<usize> {
        connection.execute(
            "INSERT INTO job (job_id, project_id, kind, status, correlation_id, revision, created_at, updated_at, progress_current, progress_total, progress_unit) VALUES (?1, ?2, 'system.mock_long', ?3, ?4, 1, ?5, ?5, 0, 100, 'step')",
            params![JOB_ID, PROJECT_ID, status, CORRELATION_ID, NOW],
        )
    }

    fn insert_job_event(
        connection: &Connection,
        event_type: &str,
        to_status: &str,
    ) -> Result<usize> {
        insert_job_event_at(connection, event_type, to_status, NOW)
    }

    fn insert_job_event_at(
        connection: &Connection,
        event_type: &str,
        to_status: &str,
        occurred_at: &str,
    ) -> Result<usize> {
        connection.execute(
            "INSERT INTO job_event (event_id, job_id, event_type, from_status, to_status, revision, progress_current, progress_total, progress_unit, occurred_at, correlation_id) VALUES (?1, ?2, ?3, NULL, ?4, 1, 0, 100, 'step', ?5, ?6)",
            params![
                EVENT_ID,
                JOB_ID,
                event_type,
                to_status,
                occurred_at,
                CORRELATION_ID
            ],
        )
    }
}
