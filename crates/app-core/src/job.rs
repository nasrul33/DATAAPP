use std::fmt::{self, Display, Formatter};

pub use teratai_contracts::generated::job_descriptor::JobDescriptor;

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

#[cfg(test)]
mod tests {
    use rusqlite::{params, Connection, Result, TransactionBehavior};

    use super::{JobError, JobErrorKind, JOB_MIGRATION};

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
