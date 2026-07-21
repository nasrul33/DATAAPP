#![doc = "Application orchestration boundary for Teratai Analytics Desktop."]

pub mod job;
pub mod job_executor;
mod project_upgrade;

pub use job::{
    JobDescriptor, JobEnqueueRequest, JobError, JobErrorKind, JobListCursor, JobPage, JobStore,
};
pub use job_executor::resource::{DurationClass, ResourceBudget, ResourceEstimate};
pub use job_executor::{
    CheckpointDecision, ClockError, ExecutionContext, ExecutorClock, HandlerOutcome, JobExecutor,
    JobExecutorConfig, JobExecutorError, JobExecutorErrorKind, JobHandler, JobHandlerError,
    JobProgress, SystemExecutorClock,
};

use std::fmt::{self, Display, Formatter};
use std::path::Path;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use teratai_contracts::generated::project_create_request::ProjectCreateRequest;
use teratai_contracts::generated::project_descriptor::ProjectDescriptor;
use teratai_contracts::generated::project_manifest::ProjectManifest;
use teratai_filesystem::{
    begin_project_creation, read_bounded, validate_project_layout, FilesystemError, ProjectLayout,
    MANIFEST_LIMIT_BYTES,
};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const PROJECT_SCHEMA_VERSION: &str = "1.0.0";
const MIN_METADATA_SCHEMA_VERSION: i64 = 1;
const METADATA_SCHEMA_VERSION: i64 = 2;
const PROJECT_MIGRATION: &str =
    include_str!("../../../migrations/metadata-sqlite/0001_project_core.sql");
const PROJECT_MIGRATIONS: [(i64, &str, &str); 2] = [
    (1, "project_core", PROJECT_MIGRATION),
    (2, "job_runtime", job::JOB_MIGRATION),
];

/// Typed failures for transactional project lifecycle operations.
#[derive(Debug)]
pub enum ProjectError {
    /// Request fields failed deterministic validation.
    InvalidRequest(String),
    /// Project layout or atomic filesystem operation failed.
    Filesystem(FilesystemError),
    /// A canonical JSON artifact could not be encoded or decoded.
    Serialization(serde_json::Error),
    /// `SQLite` migration, transaction, or validation failed.
    Database(rusqlite::Error),
    /// The project requires a newer or incompatible application version.
    IncompatibleProject(String),
    /// Manifest, `SQLite` identity, or integrity checks disagree.
    DataIntegrity(String),
    /// UTC timestamp creation or validation failed.
    Timestamp(String),
}

/// Stable adapter-facing classification that does not expose paths or raw database errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectErrorKind {
    InvalidRequest,
    TargetExists,
    RecoveryRequired,
    PermissionDenied,
    InvalidPath,
    InvalidLayout,
    ControlFileTooLarge,
    Filesystem,
    Database,
    Serialization,
    IncompatibleProject,
    DataIntegrity,
    Timestamp,
}

impl ProjectError {
    /// Return a path-safe category for IPC and presentation adapters.
    #[must_use]
    pub fn kind(&self) -> ProjectErrorKind {
        match self {
            Self::InvalidRequest(_) => ProjectErrorKind::InvalidRequest,
            Self::Filesystem(FilesystemError::AlreadyExists(_)) => ProjectErrorKind::TargetExists,
            Self::Filesystem(FilesystemError::RecoveryRequired(_)) => {
                ProjectErrorKind::RecoveryRequired
            }
            Self::Filesystem(FilesystemError::Io(error))
                if error.kind() == std::io::ErrorKind::PermissionDenied =>
            {
                ProjectErrorKind::PermissionDenied
            }
            Self::Filesystem(FilesystemError::InvalidPath(_)) => ProjectErrorKind::InvalidPath,
            Self::Filesystem(FilesystemError::InvalidLayout(_)) => ProjectErrorKind::InvalidLayout,
            Self::Filesystem(FilesystemError::FileTooLarge { .. }) => {
                ProjectErrorKind::ControlFileTooLarge
            }
            Self::Filesystem(FilesystemError::Io(_)) => ProjectErrorKind::Filesystem,
            Self::Serialization(_) => ProjectErrorKind::Serialization,
            Self::Database(_) => ProjectErrorKind::Database,
            Self::IncompatibleProject(_) => ProjectErrorKind::IncompatibleProject,
            Self::DataIntegrity(_) => ProjectErrorKind::DataIntegrity,
            Self::Timestamp(_) => ProjectErrorKind::Timestamp,
        }
    }
}

impl Display for ProjectError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(detail) => write!(formatter, "invalid project request: {detail}"),
            Self::Filesystem(error) => write!(formatter, "project filesystem failed: {error}"),
            Self::Serialization(error) => write!(formatter, "project JSON is invalid: {error}"),
            Self::Database(error) => write!(formatter, "project metadata failed: {error}"),
            Self::IncompatibleProject(detail) => {
                write!(formatter, "project version is incompatible: {detail}")
            }
            Self::DataIntegrity(detail) => write!(formatter, "project integrity failed: {detail}"),
            Self::Timestamp(detail) => write!(formatter, "project timestamp failed: {detail}"),
        }
    }
}

impl std::error::Error for ProjectError {}

impl From<FilesystemError> for ProjectError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(error)
    }
}

impl From<serde_json::Error> for ProjectError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<rusqlite::Error> for ProjectError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

/// Transactional project create/open/validate service.
#[derive(Debug, Default)]
pub struct ProjectService;

impl ProjectService {
    /// Create, migrate, validate, and atomically publish a project directory.
    ///
    /// # Errors
    ///
    /// Returns a typed error when request validation, durable creation, schema
    /// migration, integrity checks, or atomic publication fails.
    pub fn create(request: &ProjectCreateRequest) -> Result<ProjectDescriptor, ProjectError> {
        let created_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|error| ProjectError::Timestamp(error.to_string()))?;
        Self::create_at(request, &created_at)
    }

    /// Open and fully validate an existing project without mutating it.
    ///
    /// # Errors
    ///
    /// Returns a typed error for incomplete layout, incompatible versions,
    /// malformed control files, `SQLite` corruption, or identity mismatch.
    pub fn open(path: &Path) -> Result<ProjectDescriptor, ProjectError> {
        let layout = validate_project_layout(path)?;
        let (manifest, manifest_hash) = read_manifest(&layout)?;
        validate_manifest(&manifest)?;
        validate_database(&layout, &manifest, &manifest_hash)?;
        descriptor(&layout, &manifest)
    }

    /// Validate a project using the same read-only checks as open.
    ///
    /// # Errors
    ///
    /// Returns the same typed validation failures as [`Self::open`].
    pub fn validate(path: &Path) -> Result<ProjectDescriptor, ProjectError> {
        Self::open(path)
    }

    /// Explicitly upgrade a validated metadata schema-one project to schema two.
    ///
    /// Schema-two projects are returned unchanged. This method is the only
    /// project lifecycle operation allowed to mutate an existing schema-one
    /// project.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the correlation identifier, source project,
    /// migration transaction, manifest replacement, validation, or durable
    /// recovery process fails.
    pub fn upgrade(path: &Path, correlation_id: &str) -> Result<ProjectDescriptor, ProjectError> {
        project_upgrade::upgrade_project(path, correlation_id)
    }

    fn create_at(
        request: &ProjectCreateRequest,
        created_at: &str,
    ) -> Result<ProjectDescriptor, ProjectError> {
        validate_create_request(request, created_at)?;
        let manifest = ProjectManifest {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            created_at: created_at.to_owned(),
            metadata_schema_version: METADATA_SCHEMA_VERSION,
            name: request.name.clone(),
            project_id: request.project_id.clone(),
            schema_version: PROJECT_SCHEMA_VERSION.to_owned(),
        };
        let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        manifest_bytes.push(b'\n');
        let recovery_marker = serde_json::to_vec(&serde_json::json!({
            "correlation_id": request.request_id,
            "created_at": created_at,
            "project_id": request.project_id,
            "state": "CREATING"
        }))?;
        let creation = begin_project_creation(
            Path::new(&request.project_path),
            &request.project_id,
            &request.request_id,
            &recovery_marker,
        )?;
        let manifest_hash = format!("sha256:{:x}", Sha256::digest(&manifest_bytes));
        initialize_database(
            &creation.metadata_path(),
            &manifest,
            &request.request_id,
            &manifest_hash,
        )?;
        creation.write_manifest(&manifest_bytes)?;
        let layout = creation.commit()?;
        validate_database(&layout, &manifest, &manifest_hash)?;
        descriptor(&layout, &manifest)
    }
}

fn validate_create_request(
    request: &ProjectCreateRequest,
    created_at: &str,
) -> Result<(), ProjectError> {
    if !is_uuid_v7(&request.request_id) || !is_uuid_v7(&request.project_id) {
        return Err(ProjectError::InvalidRequest(
            "request_id and project_id must be lowercase UUID v7 values".to_owned(),
        ));
    }
    if request.request_id == request.project_id {
        return Err(ProjectError::InvalidRequest(
            "request_id and project_id must be distinct".to_owned(),
        ));
    }
    if request.name.is_empty()
        || request.name.trim() != request.name
        || request.name.chars().count() > 120
        || request.name.chars().any(char::is_control)
    {
        return Err(ProjectError::InvalidRequest(
            "name must contain 1-120 characters without surrounding whitespace or controls"
                .to_owned(),
        ));
    }
    validate_timestamp(created_at)
}

fn validate_manifest(manifest: &ProjectManifest) -> Result<(), ProjectError> {
    if manifest.schema_version != PROJECT_SCHEMA_VERSION {
        return Err(ProjectError::IncompatibleProject(format!(
            "manifest schema {} is unsupported",
            manifest.schema_version
        )));
    }
    if !(MIN_METADATA_SCHEMA_VERSION..=METADATA_SCHEMA_VERSION)
        .contains(&manifest.metadata_schema_version)
    {
        return Err(ProjectError::IncompatibleProject(format!(
            "metadata schema {} is unsupported",
            manifest.metadata_schema_version
        )));
    }
    if !is_uuid_v7(&manifest.project_id) {
        return Err(ProjectError::DataIntegrity(
            "manifest project_id is not UUID v7".to_owned(),
        ));
    }
    if manifest.name.is_empty()
        || manifest.name.trim() != manifest.name
        || manifest.name.chars().count() > 120
    {
        return Err(ProjectError::DataIntegrity(
            "manifest project name is invalid".to_owned(),
        ));
    }
    validate_timestamp(&manifest.created_at)
}

fn initialize_database(
    path: &Path,
    manifest: &ProjectManifest,
    correlation_id: &str,
    manifest_hash: &str,
) -> Result<(), ProjectError> {
    let mut connection = Connection::open(path)?;
    connection.pragma_update(None, "foreign_keys", true)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (version, name, migration) in PROJECT_MIGRATIONS {
        transaction.execute_batch(migration)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![version, name, manifest.created_at],
        )?;
    }
    transaction.execute(
        "INSERT INTO project (singleton, project_id, name, created_at, app_version, manifest_schema_version) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
        params![
            manifest.project_id,
            manifest.name,
            manifest.created_at,
            manifest.app_version,
            manifest.schema_version
        ],
    )?;
    transaction.execute(
        "INSERT INTO audit_event (event_id, actor, action, target_type, target_id, before_hash, after_hash, occurred_at, correlation_id) VALUES (?1, 'local-user', 'project.created', 'project', ?2, NULL, ?3, ?4, ?1)",
        params![
            correlation_id,
            manifest.project_id,
            manifest_hash,
            manifest.created_at
        ],
    )?;
    transaction.commit()?;
    connection.close().map_err(|(_, error)| error)?;
    Ok(())
}

fn validate_database(
    layout: &ProjectLayout,
    manifest: &ProjectManifest,
    manifest_hash: &str,
) -> Result<(), ProjectError> {
    let connection = Connection::open_with_flags(
        layout.metadata_path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.pragma_update(None, "foreign_keys", true)?;
    let schema_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if !(MIN_METADATA_SCHEMA_VERSION..=METADATA_SCHEMA_VERSION).contains(&schema_version) {
        return Err(ProjectError::IncompatibleProject(format!(
            "SQLite metadata schema {schema_version} is unsupported"
        )));
    }
    if schema_version != manifest.metadata_schema_version {
        return Err(ProjectError::DataIntegrity(format!(
            "manifest metadata schema {} differs from SQLite schema {schema_version}",
            manifest.metadata_schema_version
        )));
    }
    validate_migration_history(&connection, schema_version)?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(ProjectError::DataIntegrity(format!(
            "SQLite integrity_check returned {integrity}"
        )));
    }
    let stored = connection.query_row(
        "SELECT project_id, name, created_at, app_version, manifest_schema_version FROM project WHERE singleton = 1",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    )?;
    let expected = (
        manifest.project_id.clone(),
        manifest.name.clone(),
        manifest.created_at.clone(),
        manifest.app_version.clone(),
        manifest.schema_version.clone(),
    );
    if stored != expected {
        return Err(ProjectError::DataIntegrity(
            "manifest identity differs from SQLite project identity".to_owned(),
        ));
    }
    let audit_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM audit_event", [], |row| row.get(0))?;
    if audit_count < 1 {
        return Err(ProjectError::DataIntegrity(
            "project has no append-only audit history".to_owned(),
        ));
    }
    validate_manifest_authorization_chain(&connection, &manifest.project_id, manifest_hash)?;
    Ok(())
}

fn validate_manifest_authorization_chain(
    connection: &Connection,
    project_id: &str,
    manifest_hash: &str,
) -> Result<(), ProjectError> {
    let mut statement = connection.prepare(
        "SELECT sequence, action, before_hash, after_hash
         FROM audit_event
         WHERE target_type = 'project' AND target_id = ?1
         ORDER BY sequence",
    )?;
    let rows = statement.query_map([project_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;

    let mut previous_hash: Option<String> = None;
    let mut previous_sequence = 0_i64;
    let mut chain_length = 0_usize;
    for row in rows {
        let (sequence, action, before_hash, after_hash) = row?;
        if previous_sequence != 0 && sequence <= previous_sequence {
            return Err(ProjectError::DataIntegrity(
                "manifest authorization chain is not strictly increasing".to_owned(),
            ));
        }
        let after_hash = after_hash.filter(|hash| !hash.is_empty()).ok_or_else(|| {
            ProjectError::DataIntegrity(
                "manifest authorization chain contains an empty after hash".to_owned(),
            )
        })?;

        match chain_length {
            0 if sequence == 1 && action == "project.created" && before_hash.is_none() => {}
            0 => {
                return Err(ProjectError::DataIntegrity(
                    "manifest authorization chain must begin with project.created".to_owned(),
                ));
            }
            _ if action != "project.metadata_migrated" => {
                return Err(ProjectError::DataIntegrity(
                    "manifest authorization chain contains an unknown action".to_owned(),
                ));
            }
            _ if before_hash.as_deref() != previous_hash.as_deref() => {
                return Err(ProjectError::DataIntegrity(
                    "manifest authorization chain contains a broken link".to_owned(),
                ));
            }
            _ => {}
        }

        previous_hash = Some(after_hash);
        previous_sequence = sequence;
        chain_length += 1;
    }

    if chain_length == 0 || previous_hash.as_deref() != Some(manifest_hash) {
        return Err(ProjectError::DataIntegrity(
            "current manifest is not authorized by the audit chain".to_owned(),
        ));
    }
    Ok(())
}

fn validate_migration_history(
    connection: &Connection,
    schema_version: i64,
) -> Result<(), ProjectError> {
    let (migration_count, maximum_migration): (i64, i64) = connection.query_row(
        "SELECT COUNT(*), COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if migration_count != schema_version || maximum_migration != schema_version {
        return Err(ProjectError::DataIntegrity(
            "metadata migration history does not match the declared schema".to_owned(),
        ));
    }
    for (version, expected_name, _) in PROJECT_MIGRATIONS
        .iter()
        .take(usize::try_from(schema_version).unwrap_or(0))
    {
        let actual_name: Option<String> = connection
            .query_row(
                "SELECT name FROM schema_migrations WHERE version = ?1",
                [version],
                |row| row.get(0),
            )
            .optional()?;
        if actual_name.as_deref() != Some(expected_name) {
            return Err(ProjectError::DataIntegrity(
                "metadata migration history contains an unexpected migration".to_owned(),
            ));
        }
    }
    Ok(())
}

fn read_manifest(layout: &ProjectLayout) -> Result<(ProjectManifest, String), ProjectError> {
    let bytes = read_bounded(&layout.manifest_path(), MANIFEST_LIMIT_BYTES)?;
    let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
    Ok((serde_json::from_slice(&bytes)?, hash))
}

fn descriptor(
    layout: &ProjectLayout,
    manifest: &ProjectManifest,
) -> Result<ProjectDescriptor, ProjectError> {
    let project_path = layout
        .root()
        .to_str()
        .ok_or_else(|| ProjectError::InvalidRequest("project path is not UTF-8".to_owned()))?;
    Ok(ProjectDescriptor {
        created_at: manifest.created_at.clone(),
        metadata_schema_version: manifest.metadata_schema_version,
        name: manifest.name.clone(),
        project_id: manifest.project_id.clone(),
        project_path: project_path.to_owned(),
        schema_version: manifest.schema_version.clone(),
    })
}

fn validate_timestamp(value: &str) -> Result<(), ProjectError> {
    if !value.ends_with('Z') {
        return Err(ProjectError::Timestamp(
            "timestamp must use UTC Z notation".to_owned(),
        ));
    }
    OffsetDateTime::parse(value, &Rfc3339)
        .map(|_| ())
        .map_err(|error| ProjectError::Timestamp(error.to_string()))
}

pub(crate) fn is_uuid_v7(value: &str) -> bool {
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
                || matches!(byte, b'a'..=b'f')
        })
}

/// Identifies this crate as an initialized workspace component.
#[must_use]
pub const fn component_name() -> &'static str {
    "app-core"
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000100";
    const REQUEST_ID: &str = "00000000-0000-7000-8000-000000000101";
    const CREATED_AT: &str = "2026-07-20T12:00:00Z";

    fn test_parent(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "teratai-app-core-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn request(parent: &Path) -> ProjectCreateRequest {
        ProjectCreateRequest {
            name: "Audit Belanja 2026".to_owned(),
            project_id: PROJECT_ID.to_owned(),
            project_path: parent
                .join("Audit Belanja 2026.teratai")
                .to_string_lossy()
                .into_owned(),
            request_id: REQUEST_ID.to_owned(),
        }
    }

    fn create_schema_one_fixture(label: &str) -> PathBuf {
        let parent = test_parent(label);
        fs::create_dir(&parent).expect("test parent");
        let request = request(&parent);
        let manifest = ProjectManifest {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            created_at: CREATED_AT.to_owned(),
            metadata_schema_version: 1,
            name: request.name.clone(),
            project_id: request.project_id.clone(),
            schema_version: PROJECT_SCHEMA_VERSION.to_owned(),
        };
        let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("encode manifest");
        manifest_bytes.push(b'\n');
        let manifest_hash = format!("sha256:{:x}", Sha256::digest(&manifest_bytes));
        let creation = begin_project_creation(
            Path::new(&request.project_path),
            &request.project_id,
            &request.request_id,
            b"{}",
        )
        .expect("begin schema-one fixture");
        let mut connection = Connection::open(creation.metadata_path()).expect("open metadata");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("start schema-one transaction");
        transaction
            .execute_batch(PROJECT_MIGRATION)
            .expect("apply project migration");
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (1, 'project_core', ?1)",
                [CREATED_AT],
            )
            .expect("record project migration");
        transaction
            .execute(
                "INSERT INTO project (singleton, project_id, name, created_at, app_version, manifest_schema_version) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
                params![
                    manifest.project_id,
                    manifest.name,
                    manifest.created_at,
                    manifest.app_version,
                    manifest.schema_version
                ],
            )
            .expect("insert project identity");
        transaction
            .execute(
                "INSERT INTO audit_event (event_id, actor, action, target_type, target_id, before_hash, after_hash, occurred_at, correlation_id) VALUES (?1, 'local-user', 'project.created', 'project', ?2, NULL, ?3, ?4, ?1)",
                params![
                    request.request_id,
                    request.project_id,
                    manifest_hash,
                    CREATED_AT
                ],
            )
            .expect("insert initial audit event");
        transaction.commit().expect("commit schema-one fixture");
        connection.close().expect("close schema-one metadata");
        creation
            .write_manifest(&manifest_bytes)
            .expect("write schema-one manifest");
        creation
            .commit()
            .expect("publish schema-one fixture")
            .root()
            .to_path_buf()
    }

    fn create_project_fixture(label: &str) -> (ProjectDescriptor, PathBuf) {
        let parent = test_parent(label);
        fs::create_dir(&parent).expect("test parent");
        let descriptor =
            ProjectService::create_at(&request(&parent), CREATED_AT).expect("create project");
        let path = PathBuf::from(&descriptor.project_path);
        (descriptor, path)
    }

    fn create_newer_schema_fixture(version: i64) -> PathBuf {
        let (_, path) = create_project_fixture("newer-schema");
        let manifest_path = path.join("manifest.json");
        let mut manifest: ProjectManifest =
            serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
                .expect("decode manifest");
        manifest.metadata_schema_version = version;
        let mut bytes = serde_json::to_vec_pretty(&manifest).expect("encode newer manifest");
        bytes.push(b'\n');
        fs::write(manifest_path, bytes).expect("write newer manifest");
        path
    }

    fn control_file_bytes(path: &Path) -> (Vec<u8>, Vec<u8>) {
        (
            fs::read(path.join("manifest.json")).expect("read manifest bytes"),
            fs::read(path.join("metadata.sqlite")).expect("read metadata bytes"),
        )
    }

    fn sqlite_user_version(path: &Path) -> i64 {
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read user_version")
    }

    fn sqlite_migrations(path: &Path) -> Vec<(i64, String)> {
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        let mut statement = connection
            .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
            .expect("prepare migration query");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query migrations")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect migrations")
    }

    #[test]
    fn opens_schema_one_without_mutation() {
        let path = create_schema_one_fixture("readonly-v1");
        let before = control_file_bytes(&path);

        assert_eq!(
            ProjectService::open(&path)
                .expect("open schema one")
                .metadata_schema_version,
            1
        );
        assert_eq!(control_file_bytes(&path), before);
        assert_eq!(
            ProjectService::validate(&path)
                .expect("validate schema one")
                .metadata_schema_version,
            1
        );
        assert_eq!(control_file_bytes(&path), before);

        fs::remove_dir_all(path.parent().expect("fixture parent")).expect("test cleanup");
    }

    #[test]
    fn creates_schema_two_and_rejects_newer_schema() {
        let (descriptor, path) = create_project_fixture("schema-v2");

        assert_eq!(descriptor.metadata_schema_version, 2);
        assert_eq!(sqlite_user_version(&path), 2);
        assert_eq!(
            sqlite_migrations(&path),
            vec![
                (1, "project_core".to_owned()),
                (2, "job_runtime".to_owned())
            ]
        );
        let before_schema_two_open = control_file_bytes(&path);
        assert_eq!(
            ProjectService::open(&path)
                .expect("open schema two")
                .metadata_schema_version,
            2
        );
        assert_eq!(control_file_bytes(&path), before_schema_two_open);

        let newer = create_newer_schema_fixture(3);
        let before_newer_rejection = control_file_bytes(&newer);
        assert!(matches!(
            ProjectService::open(&newer),
            Err(ProjectError::IncompatibleProject(_))
        ));
        assert_eq!(control_file_bytes(&newer), before_newer_rejection);

        fs::remove_dir_all(path.parent().expect("fixture parent")).expect("test cleanup");
        fs::remove_dir_all(newer.parent().expect("newer fixture parent")).expect("test cleanup");
    }

    #[test]
    fn rejects_missing_schema_migration_record() {
        let (_, path) = create_project_fixture("missing-migration-record");
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute("DELETE FROM schema_migrations WHERE version = 2", [])
            .expect("remove migration record");
        drop(connection);

        assert!(matches!(
            ProjectService::open(&path),
            Err(ProjectError::DataIntegrity(_))
        ));

        fs::remove_dir_all(path.parent().expect("fixture parent")).expect("test cleanup");
    }

    #[test]
    fn creates_reopens_and_validates_transactional_project() {
        let parent = test_parent("lifecycle");
        fs::create_dir(&parent).expect("test parent");
        let request = request(&parent);

        let created = ProjectService::create_at(&request, CREATED_AT).expect("create project");
        let reopened =
            ProjectService::open(Path::new(&created.project_path)).expect("open project");

        assert_eq!(created, reopened);
        assert_eq!(created.metadata_schema_version, 2);
        let database = Connection::open(Path::new(&created.project_path).join("metadata.sqlite"))
            .expect("open metadata");
        let audit_count: i64 = database
            .query_row("SELECT COUNT(*) FROM audit_event", [], |row| row.get(0))
            .expect("audit count");
        assert_eq!(audit_count, 1);
        assert!(database.execute("DELETE FROM audit_event", []).is_err());
        drop(database);
        fs::remove_dir_all(parent).expect("test cleanup");
    }

    #[test]
    fn detects_manifest_and_database_identity_mismatch() {
        let parent = test_parent("tamper");
        fs::create_dir(&parent).expect("test parent");
        let request = request(&parent);
        let created = ProjectService::create_at(&request, CREATED_AT).expect("create project");
        let manifest_path = Path::new(&created.project_path).join("manifest.json");
        let mut manifest: ProjectManifest =
            serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
                .expect("decode manifest");
        manifest.name = "Nama yang diubah diam-diam".to_owned();
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("encode manifest"),
        )
        .expect("tamper manifest");

        assert!(matches!(
            ProjectService::validate(Path::new(&created.project_path)),
            Err(ProjectError::DataIntegrity(_))
        ));
        fs::remove_dir_all(parent).expect("test cleanup");
    }

    #[test]
    fn detects_additive_manifest_tampering_from_audit_hash() {
        let parent = test_parent("manifest-hash");
        fs::create_dir(&parent).expect("test parent");
        let request = request(&parent);
        let created = ProjectService::create_at(&request, CREATED_AT).expect("create project");
        let manifest_path = Path::new(&created.project_path).join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
                .expect("decode manifest");
        manifest["unreviewed_field"] = serde_json::json!(true);
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("encode manifest"),
        )
        .expect("tamper manifest");

        assert!(matches!(
            ProjectService::validate(Path::new(&created.project_path)),
            Err(ProjectError::DataIntegrity(_))
        ));
        fs::remove_dir_all(parent).expect("test cleanup");
    }

    #[test]
    fn refuses_to_overwrite_interrupted_project_creation() {
        let parent = test_parent("recovery");
        fs::create_dir(&parent).expect("test parent");
        let request = request(&parent);
        let incomplete = begin_project_creation(
            Path::new(&request.project_path),
            &request.project_id,
            &request.request_id,
            b"{}",
        )
        .expect("create interrupted staging");

        assert!(matches!(
            ProjectService::create_at(&request, CREATED_AT),
            Err(ProjectError::Filesystem(FilesystemError::RecoveryRequired(
                _
            )))
        ));
        drop(incomplete);
        fs::remove_dir_all(parent).expect("test cleanup");
    }

    #[test]
    fn rejects_invalid_identifiers_before_writing() {
        let parent = test_parent("invalid");
        fs::create_dir(&parent).expect("test parent");
        let mut request = request(&parent);
        request.request_id = "not-a-uuid".to_owned();

        assert!(matches!(
            ProjectService::create_at(&request, CREATED_AT),
            Err(ProjectError::InvalidRequest(_))
        ));
        assert_eq!(fs::read_dir(&parent).expect("read parent").count(), 0);
        fs::remove_dir_all(parent).expect("test cleanup");
    }

    #[test]
    fn classifies_errors_without_exposing_sensitive_details() {
        let permission = ProjectError::Filesystem(FilesystemError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "D:\\sensitive\\project.teratai",
        )));
        let recovery = ProjectError::Filesystem(FilesystemError::RecoveryRequired(PathBuf::from(
            "D:\\sensitive\\marker.json",
        )));

        assert_eq!(permission.kind(), ProjectErrorKind::PermissionDenied);
        assert_eq!(recovery.kind(), ProjectErrorKind::RecoveryRequired);
    }

    #[test]
    fn exposes_component_name() {
        assert_eq!(component_name(), "app-core");
    }
}
