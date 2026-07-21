use std::path::Path;

use rusqlite::{params, Connection, TransactionBehavior};
use sha2::{Digest, Sha256};
use teratai_contracts::generated::project_descriptor::ProjectDescriptor;
use teratai_contracts::generated::project_manifest::ProjectManifest;
use teratai_filesystem::{
    begin_project_upgrade, read_bounded, validate_project_layout, ProjectLayout,
    ProjectUpgradeBackupProof, MANIFEST_LIMIT_BYTES,
};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::{
    descriptor, is_uuid_v7, read_manifest, validate_database, validate_manifest, ProjectError,
    METADATA_SCHEMA_VERSION,
};

#[cfg(not(test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpgradeFault {
    None,
}

#[derive(Clone, Copy)]
enum UpgradeStage {
    BackupPrepared,
    DatabaseMigrated,
    ManifestPublished,
    ValidatedReadyToCommit,
}

struct UpgradeMarkerContext<'a> {
    manifest: &'a ProjectManifest,
    correlation_id: &'a str,
    original_manifest_hash: &'a str,
    target_manifest_hash: &'a str,
    backups: &'a ProjectUpgradeBackupProof,
}

impl UpgradeMarkerContext<'_> {
    fn bytes(&self, stage: UpgradeStage) -> Result<Vec<u8>, ProjectError> {
        upgrade_marker_bytes(
            self.manifest,
            self.correlation_id,
            self.original_manifest_hash,
            self.target_manifest_hash,
            self.backups,
            stage,
        )
    }
}

impl UpgradeStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::BackupPrepared => "BACKUP_PREPARED",
            Self::DatabaseMigrated => "DATABASE_MIGRATED",
            Self::ManifestPublished => "MANIFEST_PUBLISHED",
            Self::ValidatedReadyToCommit => "VALIDATED_READY_TO_COMMIT",
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpgradeFault {
    None,
    AfterBackupPrepared,
    AfterDatabaseCommit,
    AfterManifestPublished,
    AfterValidation,
    AfterManifestWriteCorruption,
    DuringRestoreVerification,
}

pub(super) fn upgrade_project(
    path: &Path,
    correlation_id: &str,
) -> Result<ProjectDescriptor, ProjectError> {
    upgrade_project_inner(path, correlation_id, UpgradeFault::None)
}

fn upgrade_project_inner(
    path: &Path,
    correlation_id: &str,
    fault: UpgradeFault,
) -> Result<ProjectDescriptor, ProjectError> {
    #[cfg(not(test))]
    let _ = fault;
    if !is_uuid_v7(correlation_id) {
        return Err(ProjectError::InvalidRequest(
            "correlation_id must be a lowercase UUID v7 value".to_owned(),
        ));
    }

    let layout = validate_project_layout(path)?;
    let (manifest, before_hash) = read_manifest(&layout)?;
    validate_manifest(&manifest)?;
    validate_database(&layout, &manifest, &before_hash)?;
    let before_descriptor = descriptor(&layout, &manifest)?;
    if manifest.metadata_schema_version == METADATA_SCHEMA_VERSION {
        return Ok(before_descriptor);
    }

    let mut upgraded_manifest = manifest.clone();
    upgraded_manifest.metadata_schema_version = METADATA_SCHEMA_VERSION;
    let mut upgraded_manifest_bytes = serde_json::to_vec_pretty(&upgraded_manifest)?;
    upgraded_manifest_bytes.push(b'\n');
    let after_hash = format!("sha256:{:x}", Sha256::digest(&upgraded_manifest_bytes));
    let migrated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| ProjectError::Timestamp(error.to_string()))?;
    let mut backup_proof = None;
    let mut marker = Vec::new();
    let mut guard = begin_project_upgrade::<ProjectError, _>(&layout, correlation_id, |proof| {
        backup_proof = Some(proof.clone());
        marker = upgrade_marker_bytes(
            &manifest,
            correlation_id,
            &before_hash,
            &after_hash,
            proof,
            UpgradeStage::BackupPrepared,
        )?;
        Ok(marker.clone())
    })?;
    let backup_proof = backup_proof.ok_or_else(|| {
        ProjectError::DataIntegrity("upgrade backup proof was not retained".to_owned())
    })?;
    let marker_context = UpgradeMarkerContext {
        manifest: &manifest,
        correlation_id,
        original_manifest_hash: &before_hash,
        target_manifest_hash: &after_hash,
        backups: &backup_proof,
    };
    #[cfg(test)]
    inject_after_backup_prepared(fault, &marker)?;

    let upgrade_result = (|| -> Result<ProjectDescriptor, ProjectError> {
        migrate_database(
            &layout,
            &manifest.project_id,
            correlation_id,
            &migrated_at,
            &before_hash,
            &after_hash,
        )?;
        marker = marker_context.bytes(UpgradeStage::DatabaseMigrated)?;
        guard.write_marker(&marker)?;
        #[cfg(test)]
        inject_after_database_commit(fault, &marker)?;
        guard.write_manifest(&upgraded_manifest_bytes)?;
        marker = marker_context.bytes(UpgradeStage::ManifestPublished)?;
        guard.write_marker(&marker)?;
        #[cfg(test)]
        inject_after_manifest_published(fault, &marker)?;
        #[cfg(test)]
        inject_after_manifest_write(fault, &layout);
        let (written_manifest, written_hash) = read_validated_upgraded_manifest(
            &layout,
            &upgraded_manifest_bytes,
            &after_hash,
            &manifest.project_id,
        )?;
        validate_database(&layout, &written_manifest, &written_hash)?;
        let upgraded_descriptor = descriptor(&layout, &written_manifest)?;
        marker = marker_context.bytes(UpgradeStage::ValidatedReadyToCommit)?;
        guard.write_marker(&marker)?;
        #[cfg(test)]
        inject_after_validation(fault, &marker)?;
        Ok(upgraded_descriptor)
    })();

    match upgrade_result {
        Ok(upgraded_descriptor) => {
            guard.commit()?;
            Ok(upgraded_descriptor)
        }
        Err(upgrade_error) => {
            #[cfg(test)]
            inject_restore_fault(fault, &layout);
            match guard.restore() {
                Ok(()) => Err(upgrade_error),
                Err(restore_error) => Err(ProjectError::Filesystem(restore_error)),
            }
        }
    }
}

fn upgrade_marker_bytes(
    manifest: &ProjectManifest,
    correlation_id: &str,
    original_manifest_hash: &str,
    target_manifest_hash: &str,
    backups: &ProjectUpgradeBackupProof,
    stage: UpgradeStage,
) -> Result<Vec<u8>, ProjectError> {
    Ok(serde_json::to_vec(&serde_json::json!({
        "backups": {
            "manifest": {
                "content_digest": backups.manifest.content_digest.as_str(),
                "length": backups.manifest.length,
                "name": "manifest-schema-1.json.backup"
            },
            "metadata": {
                "content_digest": backups.metadata.content_digest.as_str(),
                "length": backups.metadata.length,
                "name": "metadata-schema-1.sqlite.backup"
            }
        },
        "correlation_id": correlation_id,
        "original_manifest_hash": original_manifest_hash,
        "project_id": manifest.project_id.as_str(),
        "source_metadata_schema_version": manifest.metadata_schema_version,
        "stage": stage.as_str(),
        "target_manifest_hash": target_manifest_hash,
        "target_metadata_schema_version": METADATA_SCHEMA_VERSION,
        "version": 1
    }))?)
}

fn read_validated_upgraded_manifest(
    layout: &ProjectLayout,
    expected_bytes: &[u8],
    expected_hash: &str,
    expected_project_id: &str,
) -> Result<(ProjectManifest, String), ProjectError> {
    let written_bytes = read_bounded(&layout.manifest_path(), MANIFEST_LIMIT_BYTES)?;
    let written_hash = format!("sha256:{:x}", Sha256::digest(&written_bytes));
    let written_manifest: ProjectManifest =
        serde_json::from_slice(&written_bytes).map_err(|_| {
            ProjectError::DataIntegrity("upgraded manifest could not be decoded safely".to_owned())
        })?;
    validate_manifest(&written_manifest)?;

    if written_bytes != expected_bytes
        || written_hash != expected_hash
        || written_manifest.metadata_schema_version != METADATA_SCHEMA_VERSION
        || written_manifest.project_id != expected_project_id
    {
        return Err(ProjectError::DataIntegrity(
            "upgraded manifest differs from the authorized replacement".to_owned(),
        ));
    }

    Ok((written_manifest, written_hash))
}

fn migrate_database(
    layout: &ProjectLayout,
    project_id: &str,
    correlation_id: &str,
    migrated_at: &str,
    before_hash: &str,
    after_hash: &str,
) -> Result<(), ProjectError> {
    let mut connection = Connection::open(layout.metadata_path())?;
    connection.pragma_update(None, "foreign_keys", true)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let schema_version: i64 =
        transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema_version != 1 {
        return Err(ProjectError::DataIntegrity(
            "metadata schema changed before the upgrade transaction".to_owned(),
        ));
    }
    transaction.execute_batch(crate::job::JOB_MIGRATION)?;
    transaction.execute(
        "INSERT INTO schema_migrations (version, name, applied_at) VALUES (2, 'job_runtime', ?1)",
        [migrated_at],
    )?;
    transaction.execute(
        "INSERT INTO audit_event (event_id, actor, action, target_type, target_id, before_hash, after_hash, occurred_at, correlation_id)
         VALUES (?1, 'local-user', 'project.metadata_migrated', 'project', ?2, ?3, ?4, ?5, ?1)",
        params![correlation_id, project_id, before_hash, after_hash, migrated_at],
    )?;
    transaction.commit()?;
    connection.close().map_err(|(_, error)| error)?;
    Ok(())
}

#[cfg(test)]
thread_local! {
    static CAPTURED_MARKER: std::cell::RefCell<Option<Vec<u8>>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
fn capture_marker(contents: &[u8]) {
    CAPTURED_MARKER.with(|captured| captured.replace(Some(contents.to_vec())));
}

#[cfg(test)]
fn take_captured_marker() -> Vec<u8> {
    CAPTURED_MARKER.with(|captured| captured.replace(None).expect("upgrade marker was captured"))
}

#[cfg(test)]
fn inject_after_backup_prepared(fault: UpgradeFault, marker: &[u8]) -> Result<(), ProjectError> {
    if fault == UpgradeFault::AfterBackupPrepared {
        capture_marker(marker);
        return Err(ProjectError::DataIntegrity(
            "injected project upgrade failure".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn inject_after_database_commit(fault: UpgradeFault, marker: &[u8]) -> Result<(), ProjectError> {
    if matches!(
        fault,
        UpgradeFault::AfterDatabaseCommit | UpgradeFault::DuringRestoreVerification
    ) {
        capture_marker(marker);
        return Err(ProjectError::DataIntegrity(
            "injected project upgrade failure".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn inject_after_manifest_published(fault: UpgradeFault, marker: &[u8]) -> Result<(), ProjectError> {
    if fault == UpgradeFault::AfterManifestPublished {
        capture_marker(marker);
        return Err(ProjectError::DataIntegrity(
            "injected project upgrade failure".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn inject_after_validation(fault: UpgradeFault, marker: &[u8]) -> Result<(), ProjectError> {
    if fault == UpgradeFault::AfterValidation {
        capture_marker(marker);
        return Err(ProjectError::DataIntegrity(
            "injected project upgrade failure".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn inject_restore_fault(fault: UpgradeFault, layout: &ProjectLayout) {
    if fault == UpgradeFault::DuringRestoreVerification {
        let metadata_path = layout.metadata_path();
        let _ = std::fs::remove_file(&metadata_path);
        std::fs::create_dir(&metadata_path).expect("inject deterministic restore failure");
    }
}

#[cfg(test)]
fn inject_after_manifest_write(fault: UpgradeFault, layout: &ProjectLayout) {
    if fault == UpgradeFault::AfterManifestWriteCorruption {
        std::fs::write(layout.manifest_path(), b"{corrupted-after-write")
            .expect("inject deterministic manifest corruption");
    }
}

#[cfg(test)]
fn upgrade_with_fault(
    path: &Path,
    correlation_id: &str,
    fault: UpgradeFault,
) -> Result<ProjectDescriptor, ProjectError> {
    upgrade_project_inner(path, correlation_id, fault)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::{params, Connection, TransactionBehavior};
    use sha2::{Digest, Sha256};
    use teratai_contracts::generated::project_create_request::ProjectCreateRequest;
    use teratai_contracts::generated::project_manifest::ProjectManifest;
    use teratai_filesystem::{begin_project_creation, FilesystemError};

    use super::{take_captured_marker, upgrade_with_fault, UpgradeFault};
    use crate::{ProjectError, ProjectService, PROJECT_MIGRATION, PROJECT_SCHEMA_VERSION};

    const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000110";
    const CREATE_CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000111";
    const UPGRADE_CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000112";
    const CREATED_AT: &str = "2026-07-21T01:00:00Z";

    fn test_parent(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "teratai-project-upgrade-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn create_schema_one_fixture(label: &str) -> PathBuf {
        let parent = test_parent(label);
        fs::create_dir(&parent).expect("test parent");
        let target = parent.join("Upgrade Fixture.teratai");
        let request = ProjectCreateRequest {
            name: "Audit Upgrade 2026".to_owned(),
            project_id: PROJECT_ID.to_owned(),
            project_path: target.to_string_lossy().into_owned(),
            request_id: CREATE_CORRELATION_ID.to_owned(),
        };
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
        let creation =
            begin_project_creation(&target, &request.project_id, &request.request_id, b"{}")
                .expect("begin fixture");
        let mut connection = Connection::open(creation.metadata_path()).expect("open metadata");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("begin fixture transaction");
        transaction
            .execute_batch(PROJECT_MIGRATION)
            .expect("apply schema one");
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (1, 'project_core', ?1)",
                [CREATED_AT],
            )
            .expect("record migration");
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
            .expect("insert project");
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
            .expect("insert creation audit");
        transaction.commit().expect("commit fixture");
        connection.close().expect("close metadata");
        creation
            .write_manifest(&manifest_bytes)
            .expect("write manifest");
        creation
            .commit()
            .expect("publish fixture")
            .root()
            .to_owned()
    }

    fn control_file_bytes(path: &Path) -> (Vec<u8>, Vec<u8>) {
        (
            fs::read(path.join("manifest.json")).expect("read manifest"),
            fs::read(path.join("metadata.sqlite")).expect("read metadata"),
        )
    }

    fn audit_rows(path: &Path) -> Vec<(i64, String, Option<String>, Option<String>)> {
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        let mut statement = connection
            .prepare(
                "SELECT sequence, action, before_hash, after_hash FROM audit_event ORDER BY sequence",
            )
            .expect("prepare audit query");
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .expect("query audits")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect audits")
    }

    fn cleanup(path: &Path) {
        fs::remove_dir_all(path.parent().expect("fixture parent")).expect("test cleanup");
    }

    #[test]
    fn upgrade_preserves_identity_and_prior_audit() {
        let path = create_schema_one_fixture("preserves");
        let before = ProjectService::open(&path).expect("open schema one");
        let audit_before = audit_rows(&path);

        let after = ProjectService::upgrade(&path, UPGRADE_CORRELATION_ID).expect("upgrade");

        assert_eq!(after.project_id, before.project_id);
        assert_eq!(after.metadata_schema_version, 2);
        let audit_after = audit_rows(&path);
        assert_eq!(audit_after.len(), audit_before.len() + 1);
        assert_eq!(&audit_after[..audit_before.len()], audit_before.as_slice());
        assert_eq!(
            audit_after.last().expect("migration audit").1,
            "project.metadata_migrated"
        );
        assert_eq!(ProjectService::open(&path).expect("reopen upgraded"), after);
        cleanup(&path);
    }

    #[test]
    fn upgrade_failure_restores_v1_or_requires_recovery() {
        let restored = create_schema_one_fixture("restored");
        let restored_before = control_file_bytes(&restored);
        assert!(upgrade_with_fault(
            &restored,
            UPGRADE_CORRELATION_ID,
            UpgradeFault::AfterDatabaseCommit,
        )
        .is_err());
        assert_eq!(control_file_bytes(&restored), restored_before);
        assert_eq!(
            ProjectService::open(&restored)
                .expect("restored project")
                .metadata_schema_version,
            1
        );
        cleanup(&restored);

        let unproven = create_schema_one_fixture("unproven");
        assert!(upgrade_with_fault(
            &unproven,
            UPGRADE_CORRELATION_ID,
            UpgradeFault::DuringRestoreVerification,
        )
        .is_err());
        assert!(matches!(
            ProjectService::open(&unproven),
            Err(ProjectError::Filesystem(FilesystemError::RecoveryRequired(
                _
            )))
        ));
        cleanup(&unproven);
    }

    #[test]
    fn upgrade_marker_binds_backups_and_advances_every_irreversible_stage() {
        let cases = [
            (UpgradeFault::AfterBackupPrepared, "BACKUP_PREPARED"),
            (UpgradeFault::AfterDatabaseCommit, "DATABASE_MIGRATED"),
            (UpgradeFault::AfterManifestPublished, "MANIFEST_PUBLISHED"),
            (UpgradeFault::AfterValidation, "VALIDATED_READY_TO_COMMIT"),
        ];

        for (index, (fault, expected_stage)) in cases.into_iter().enumerate() {
            let path = create_schema_one_fixture(&format!("marker-stage-{index}"));
            let original_manifest = fs::read(path.join("manifest.json")).expect("read manifest");
            let original_manifest_hash = format!("sha256:{:x}", Sha256::digest(original_manifest));

            assert!(upgrade_with_fault(&path, UPGRADE_CORRELATION_ID, fault).is_err());
            let marker: serde_json::Value =
                serde_json::from_slice(&take_captured_marker()).expect("decode marker");

            assert_eq!(marker["project_id"], PROJECT_ID);
            assert_eq!(marker["correlation_id"], UPGRADE_CORRELATION_ID);
            assert_eq!(marker["source_metadata_schema_version"], 1);
            assert_eq!(marker["target_metadata_schema_version"], 2);
            assert_eq!(marker["original_manifest_hash"], original_manifest_hash);
            assert_eq!(marker["stage"], expected_stage);
            for backup in ["metadata", "manifest"] {
                let proof = &marker["backups"][backup];
                assert!(proof["length"].as_u64().is_some_and(|length| length > 0));
                let digest = proof["content_digest"]
                    .as_str()
                    .expect("bounded backup digest");
                assert!(digest.starts_with("proof-v1:"));
                assert!(digest.len() <= 80);
            }
            cleanup(&path);
        }
    }

    #[test]
    fn corrupted_manifest_after_write_never_commits_recovery_proof() {
        let path = create_schema_one_fixture("corrupted-after-manifest-write");
        let before = control_file_bytes(&path);

        assert!(upgrade_with_fault(
            &path,
            UPGRADE_CORRELATION_ID,
            UpgradeFault::AfterManifestWriteCorruption,
        )
        .is_err());

        match ProjectService::open(&path) {
            Ok(descriptor) => {
                assert_eq!(descriptor.metadata_schema_version, 1);
                assert_eq!(control_file_bytes(&path), before);
            }
            Err(ProjectError::Filesystem(FilesystemError::RecoveryRequired(_))) => {}
            Err(error) => panic!("unexpected recovery result: {error}"),
        }
        cleanup(&path);
    }

    #[test]
    fn schema_two_upgrade_is_idempotent() {
        let path = create_schema_one_fixture("idempotent");
        let first = ProjectService::upgrade(&path, UPGRADE_CORRELATION_ID).expect("first upgrade");
        let files_after_first = control_file_bytes(&path);
        let audit_after_first = audit_rows(&path);

        let second =
            ProjectService::upgrade(&path, UPGRADE_CORRELATION_ID).expect("second upgrade");

        assert_eq!(second, first);
        assert_eq!(control_file_bytes(&path), files_after_first);
        assert_eq!(audit_rows(&path), audit_after_first);
        cleanup(&path);
    }

    #[test]
    fn invalid_correlation_id_is_rejected_without_mutation() {
        let path = create_schema_one_fixture("invalid-correlation");
        let before = control_file_bytes(&path);

        assert!(matches!(
            ProjectService::upgrade(&path, "not-a-uuid"),
            Err(ProjectError::InvalidRequest(_))
        ));
        assert_eq!(control_file_bytes(&path), before);
        cleanup(&path);
    }

    #[test]
    fn manifest_audit_chain_rejects_tampered_link() {
        let path = create_schema_one_fixture("tampered-chain");
        ProjectService::upgrade(&path, UPGRADE_CORRELATION_ID).expect("upgrade");
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute_batch("DROP TRIGGER audit_event_prevent_update;")
            .expect("simulate storage tampering");
        connection
            .execute(
                "UPDATE audit_event SET before_hash = 'sha256:tampered' WHERE action = 'project.metadata_migrated'",
                [],
            )
            .expect("tamper chain link");
        drop(connection);

        assert!(matches!(
            ProjectService::open(&path),
            Err(ProjectError::DataIntegrity(_))
        ));
        cleanup(&path);
    }

    #[test]
    fn manifest_audit_chain_rejects_unknown_manifest_action() {
        let path = create_schema_one_fixture("unknown-chain-action");
        ProjectService::upgrade(&path, UPGRADE_CORRELATION_ID).expect("upgrade");
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute_batch("DROP TRIGGER audit_event_prevent_update;")
            .expect("simulate storage tampering");
        connection
            .execute(
                "UPDATE audit_event SET action = 'project.manifest_rewritten' WHERE action = 'project.metadata_migrated'",
                [],
            )
            .expect("tamper action");
        drop(connection);

        assert!(matches!(
            ProjectService::validate(&path),
            Err(ProjectError::DataIntegrity(_))
        ));
        cleanup(&path);
    }

    #[test]
    fn manifest_audit_chain_allows_interleaved_non_project_audits() {
        let path = create_schema_one_fixture("interleaved-chain");
        ProjectService::upgrade(&path, UPGRADE_CORRELATION_ID).expect("upgrade");
        let manifest_hash = format!(
            "sha256:{:x}",
            Sha256::digest(fs::read(path.join("manifest.json")).expect("read manifest"))
        );
        let connection = Connection::open(path.join("metadata.sqlite")).expect("open metadata");
        connection
            .execute(
                "INSERT INTO audit_event (
                    event_id, actor, action, target_type, target_id,
                    before_hash, after_hash, occurred_at, correlation_id
                 ) VALUES (?1, 'local-user', 'job.progressed', 'job', ?2,
                           'sha256:before', 'sha256:after', ?3, ?1)",
                params![
                    "00000000-0000-7000-8000-000000000113",
                    "00000000-0000-7000-8000-000000000210",
                    CREATED_AT,
                ],
            )
            .expect("insert interleaved job audit");
        connection
            .execute(
                "INSERT INTO audit_event (
                    event_id, actor, action, target_type, target_id,
                    before_hash, after_hash, occurred_at, correlation_id
                 ) VALUES (?1, 'local-user', 'project.metadata_migrated', 'project', ?2,
                           ?3, ?3, ?4, ?1)",
                params![
                    "00000000-0000-7000-8000-000000000114",
                    PROJECT_ID,
                    manifest_hash,
                    CREATED_AT,
                ],
            )
            .expect("insert later manifest audit");
        drop(connection);

        ProjectService::open(&path).expect("open with interleaved global audit sequence");
        cleanup(&path);
    }
}
