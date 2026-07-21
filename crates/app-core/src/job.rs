use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Row, TransactionBehavior};
use sha2::{Digest, Sha256};
pub use teratai_contracts::generated::job_descriptor::JobDescriptor;
pub use teratai_contracts::generated::job_enqueue_request::JobEnqueueRequest;
use teratai_filesystem::{
    pin_project_metadata, validate_project_layout, PinnedProjectMetadata, ProjectLayout,
};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

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
        let descriptor = validate_schema_two_project(layout.root())?;
        let canonical_path = PathBuf::from(&descriptor.project_path);
        if canonical_path != layout.root() {
            return Err(JobError::DataIntegrity(
                "validated project path changed while opening".to_owned(),
            ));
        }
        verify_pinned_metadata(&metadata)?;
        let connection = open_connection(metadata.path(), ConnectionAccess::ReadOnly)?;
        verify_pinned_metadata(&metadata)?;
        probe_job_connection(&connection, &descriptor.project_id)?;
        verify_pinned_metadata(&metadata)?;
        drop(connection);
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

    /// Return one safe job snapshot by lowercase UUID v7 identity.
    ///
    /// # Errors
    ///
    /// Returns `InvalidRequest`, `JobNotFound`, or a safe database failure.
    pub fn get(&self, job_id: &str) -> Result<JobDescriptor, JobError> {
        validate_uuid(job_id, "job_id")?;
        let connection = self.read_connection()?;
        connection
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
            .optional()?
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
        let connection = self.read_connection()?;
        let mut items = if let Some(value) = cursor {
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
        let after_hash = snapshot_hash(&descriptor)?;

        let mut connection = self.write_connection()?;
        verify_pinned_metadata(&self.metadata)?;
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
        let refresh_result = self
            .metadata
            .refresh_after_authorized_write()
            .map_err(|_| JobError::DataIntegrity("pinned metadata identity changed".to_owned()));
        refresh_result?;
        write_result?;
        Ok(descriptor)
    }

    fn read_connection(&self) -> Result<Connection, JobError> {
        self.validated_connection(ConnectionAccess::ReadOnly)
    }

    fn write_connection(&self) -> Result<Connection, JobError> {
        self.validated_connection(ConnectionAccess::ReadWrite)
    }

    fn validated_connection(&self, access: ConnectionAccess) -> Result<Connection, JobError> {
        let layout = validated_project_layout(&self.project_path)?;
        if layout.root() != self.project_path || layout.metadata_path() != self.metadata.path() {
            return Err(JobError::DataIntegrity(
                "project control paths changed after the job store opened".to_owned(),
            ));
        }
        verify_pinned_metadata(&self.metadata)?;
        let connection = open_connection(self.metadata.path(), access)?;
        verify_pinned_metadata(&self.metadata)?;
        probe_job_connection(&connection, &self.project_id)?;
        verify_pinned_metadata(&self.metadata)?;
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

fn verify_pinned_metadata(metadata: &PinnedProjectMetadata) -> Result<(), JobError> {
    metadata
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

fn row_to_descriptor(row: &Row<'_>) -> rusqlite::Result<JobDescriptor> {
    Ok(JobDescriptor {
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
        error_retriable: row.get::<_, Option<i64>>(17)?.map(|value| value != 0),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use rusqlite::{params, Connection, Result, TransactionBehavior};
    use sha2::{Digest, Sha256};
    use teratai_contracts::generated::job_enqueue_request::JobEnqueueRequest;
    use teratai_contracts::generated::project_create_request::ProjectCreateRequest;
    use teratai_contracts::generated::project_manifest::ProjectManifest;
    use teratai_filesystem::REQUIRED_PROJECT_DIRECTORIES;

    use super::{
        event_id_at, full_project_validation_count, snapshot_hash, JobError, JobErrorKind,
        JobListCursor, JobStore, JOB_MIGRATION,
    };
    use crate::ProjectService;

    const PROJECT_MIGRATION: &str =
        include_str!("../../../migrations/metadata-sqlite/0001_project_core.sql");
    const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000100";
    const JOB_ID: &str = "00000000-0000-7000-8000-000000000210";
    const EVENT_ID: &str = "00000000-0000-7000-8000-000000000211";
    const CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000212";
    const NOW: &str = "2026-07-20T12:00:00Z";
    const INVALID_CALENDAR_TIMESTAMPS: [&str; 4] = [
        "2026-13-10T12:00:00Z",
        "2026-01-32T12:00:00Z",
        "2026-01-10T23:60:00Z",
        "2026-01-10T23:59:60Z",
    ];

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
        let connection = store.read_connection().expect("open store connection");
        let foreign_keys: i64 = connection
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .expect("read foreign key setting");

        assert_eq!(foreign_keys, 1);
        assert!(connection.is_readonly("main").expect("read access mode"));
        drop(connection);
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
