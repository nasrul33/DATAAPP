//! Test-only, genuine schema-one project fixtures for native integration tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, TransactionBehavior};
use sha2::{Digest, Sha256};
use teratai_contracts::generated::project_create_request::ProjectCreateRequest;
use teratai_contracts::generated::project_manifest::ProjectManifest;
use teratai_filesystem::{begin_project_creation, FilesystemError};
pub use teratai_filesystem::{
    install_begin_project_upgrade_fault_for_test, BeginProjectUpgradeFault,
    BeginProjectUpgradeFaultGuard,
};

use crate::{ProjectError, PROJECT_MIGRATION, PROJECT_SCHEMA_VERSION};

/// Owns a published metadata-schema-one project fixture and removes its parent
/// directory when the fixture is dropped.
#[derive(Debug)]
pub struct SchemaOneProjectFixture {
    parent: PathBuf,
    root: PathBuf,
}

impl SchemaOneProjectFixture {
    /// Return the trusted fixture project root.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Return audit actions in durable sequence order.
    ///
    /// # Errors
    ///
    /// Returns the same typed metadata error used by application code when
    /// the fixture database cannot be queried.
    pub fn audit_actions(&self) -> Result<Vec<String>, ProjectError> {
        let connection = Connection::open(self.root.join("metadata.sqlite"))?;
        let actions = {
            let mut statement =
                connection.prepare("SELECT action FROM audit_event ORDER BY sequence")?;
            let actions = statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            actions
        };
        connection
            .close()
            .map_err(|(_, error)| ProjectError::Database(error))?;
        Ok(actions)
    }

    /// Change only the `SQLite` user version to construct an activation-boundary
    /// integrity fixture after a durable schema upgrade.
    ///
    /// # Errors
    ///
    /// Returns the typed metadata failure if the fixture cannot be mutated.
    pub fn set_metadata_user_version(&self, version: i64) -> Result<(), ProjectError> {
        let connection = Connection::open(self.root.join("metadata.sqlite"))?;
        connection.pragma_update(None, "user_version", version)?;
        connection
            .close()
            .map_err(|(_, error)| ProjectError::Database(error))?;
        Ok(())
    }
}

impl Drop for SchemaOneProjectFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.parent);
    }
}

/// Create a fully durable metadata-schema-one project without invoking the
/// schema-two project creation service.
///
/// This utility is compiled only for app-core tests or consumers that opt in
/// to the `test-utils` feature. It applies migration 0001, records its schema
/// migration and creation audit event, writes the matching manifest, and then
/// publishes the project through the real filesystem creation protocol.
///
/// # Errors
///
/// Returns a typed fixture setup error without leaving a successful fixture
/// partially published.
pub fn create_schema_one_project_fixture(
    label: &str,
    project_id: &str,
    creation_request_id: &str,
    name: &str,
    created_at: &str,
) -> Result<SchemaOneProjectFixture, ProjectError> {
    if label.is_empty()
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(ProjectError::InvalidRequest(
            "schema-one fixture label is invalid".to_owned(),
        ));
    }

    let parent = fixture_parent(label);
    fs::create_dir(&parent).map_err(FilesystemError::Io)?;
    let result =
        create_schema_one_project(&parent, project_id, creation_request_id, name, created_at);
    match result {
        Ok(root) => Ok(SchemaOneProjectFixture { parent, root }),
        Err(error) => {
            let _ = fs::remove_dir_all(&parent);
            Err(error)
        }
    }
}

fn fixture_parent(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "teratai-schema-one-fixture-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn create_schema_one_project(
    parent: &Path,
    project_id: &str,
    creation_request_id: &str,
    name: &str,
    created_at: &str,
) -> Result<PathBuf, ProjectError> {
    let target = parent.join("Schema One Fixture.teratai");
    let request = ProjectCreateRequest {
        name: name.to_owned(),
        project_id: project_id.to_owned(),
        project_path: target.to_string_lossy().into_owned(),
        request_id: creation_request_id.to_owned(),
    };
    let manifest = ProjectManifest {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        created_at: created_at.to_owned(),
        metadata_schema_version: 1,
        name: request.name.clone(),
        project_id: request.project_id.clone(),
        schema_version: PROJECT_SCHEMA_VERSION.to_owned(),
    };
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    manifest_bytes.push(b'\n');
    let manifest_hash = format!("sha256:{:x}", Sha256::digest(&manifest_bytes));
    let creation =
        begin_project_creation(&target, &request.project_id, &request.request_id, b"{}")?;
    let mut connection = Connection::open(creation.metadata_path())?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(PROJECT_MIGRATION)?;
    transaction.execute(
        "INSERT INTO schema_migrations (version, name, applied_at) VALUES (1, 'project_core', ?1)",
        [created_at],
    )?;
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
            request.request_id,
            request.project_id,
            manifest_hash,
            created_at
        ],
    )?;
    transaction.commit()?;
    connection
        .close()
        .map_err(|(_, error)| ProjectError::Database(error))?;
    creation.write_manifest(&manifest_bytes)?;
    Ok(creation.commit()?.root().to_owned())
}
