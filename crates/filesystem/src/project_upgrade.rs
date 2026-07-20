use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::{
    atomic_write, validate_safe_token, FilesystemError, ProjectLayout, MANIFEST_LIMIT_BYTES,
};

pub(crate) const UPGRADE_MARKER_FILE: &str = ".project-upgrade-recovery.json";
pub(crate) const METADATA_BACKUP_FILE: &str = "metadata-schema-1.sqlite.backup";
pub(crate) const MANIFEST_BACKUP_FILE: &str = "manifest-schema-1.json.backup";

/// A durable guard for an in-place project metadata upgrade.
#[derive(Debug)]
pub struct ProjectUpgrade {
    layout: ProjectLayout,
    marker_path: PathBuf,
    metadata_backup_path: PathBuf,
    manifest_backup_path: PathBuf,
    correlation_id: String,
    committed: bool,
}

impl ProjectUpgrade {
    /// Atomically replace the bounded recovery marker after a stage change.
    ///
    /// # Errors
    ///
    /// Returns an error when the marker is missing, linked, non-regular, too
    /// large, or cannot be durably replaced.
    pub fn write_marker(&self, contents: &[u8]) -> Result<(), FilesystemError> {
        ensure_regular_file(&self.marker_path, "upgrade recovery marker")?;
        write_bounded_file(
            &self.marker_path,
            contents,
            MANIFEST_LIMIT_BYTES,
            &self.correlation_id,
        )
    }

    /// Atomically replace the bounded project manifest.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest is linked, non-regular, too large,
    /// or cannot be durably replaced.
    pub fn write_manifest(&self, contents: &[u8]) -> Result<(), FilesystemError> {
        let manifest_path = self.layout.manifest_path();
        ensure_regular_file(&manifest_path, "project manifest")?;
        write_bounded_file(
            &manifest_path,
            contents,
            MANIFEST_LIMIT_BYTES,
            &self.correlation_id,
        )
    }

    /// Restore both project control files from their durable schema-1 backups.
    ///
    /// Recovery artifacts are removed only after both restored files compare
    /// byte-for-byte with their backups. Any failed proof leaves the marker and
    /// available backups in place for explicit recovery.
    ///
    /// # Errors
    ///
    /// Returns an error when recovery artifacts are incomplete or unsafe,
    /// restoration fails, or byte verification cannot be completed.
    pub fn restore(&mut self) -> Result<(), FilesystemError> {
        if self.committed {
            return Ok(());
        }
        self.validate_recovery_artifacts()?;

        restore_regular_backup(
            &self.metadata_backup_path,
            &self.layout.metadata_path(),
            &self.correlation_id,
        )?;
        restore_regular_backup(
            &self.manifest_backup_path,
            &self.layout.manifest_path(),
            &self.correlation_id,
        )?;

        if !files_equal(&self.metadata_backup_path, &self.layout.metadata_path())?
            || !files_equal(&self.manifest_backup_path, &self.layout.manifest_path())?
        {
            return Err(FilesystemError::InvalidLayout(
                "restored project control files could not be verified".to_owned(),
            ));
        }

        self.remove_recovery_artifacts()?;
        self.committed = true;
        Ok(())
    }

    /// Commit an already-validated upgrade by clearing its recovery artifacts.
    ///
    /// The marker is removed last so normal project open remains blocked while
    /// either backup cleanup is still in progress.
    ///
    /// # Errors
    ///
    /// Returns an error when a control/recovery entry is unsafe or an artifact
    /// cannot be removed. Dropping the returned error path attempts restoration.
    pub fn commit(mut self) -> Result<(), FilesystemError> {
        ensure_regular_file(&self.layout.metadata_path(), "project metadata")?;
        ensure_regular_file(&self.layout.manifest_path(), "project manifest")?;
        self.validate_recovery_artifacts()?;
        self.remove_recovery_artifacts()?;
        self.committed = true;
        Ok(())
    }

    fn validate_recovery_artifacts(&self) -> Result<(), FilesystemError> {
        ensure_regular_directory(&self.layout.root().join("recovery"), "recovery directory")?;
        ensure_regular_file(&self.marker_path, "upgrade recovery marker")?;
        ensure_regular_file(&self.metadata_backup_path, "metadata recovery backup")?;
        ensure_regular_file(&self.manifest_backup_path, "manifest recovery backup")
    }

    fn remove_recovery_artifacts(&self) -> Result<(), FilesystemError> {
        fs::remove_file(&self.metadata_backup_path)?;
        fs::remove_file(&self.manifest_backup_path)?;
        fs::remove_file(&self.marker_path)?;
        Ok(())
    }
}

impl Drop for ProjectUpgrade {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.restore();
        }
    }
}

/// Begin a durable, recovery-marked upgrade of a validated project layout.
///
/// Both original control files are copied and synced before the recovery
/// marker is published. A failure after either backup is created intentionally
/// leaves that artifact in place so normal project validation requires
/// explicit recovery.
///
/// # Errors
///
/// Returns an error for an invalid correlation ID, oversized marker, stale or
/// unsafe layout, existing recovery artifacts, or operating-system failure.
pub fn begin_project_upgrade(
    layout: &ProjectLayout,
    correlation_id: &str,
    marker_contents: &[u8],
) -> Result<ProjectUpgrade, FilesystemError> {
    validate_safe_token(correlation_id, "correlation_id")?;
    let marker_path = layout.root().join(UPGRADE_MARKER_FILE);
    ensure_bounded(marker_contents, MANIFEST_LIMIT_BYTES, &marker_path)?;

    let recovery_directory = layout.root().join("recovery");
    ensure_regular_directory(&recovery_directory, "recovery directory")?;
    ensure_regular_file(&layout.metadata_path(), "project metadata")?;
    ensure_regular_file(&layout.manifest_path(), "project manifest")?;

    let metadata_backup_path = recovery_directory.join(METADATA_BACKUP_FILE);
    let manifest_backup_path = recovery_directory.join(MANIFEST_BACKUP_FILE);
    for artifact in [&marker_path, &metadata_backup_path, &manifest_backup_path] {
        if entry_exists_no_follow(artifact)? {
            return Err(FilesystemError::RecoveryRequired(artifact.clone()));
        }
    }

    create_durable_backup(&layout.metadata_path(), &metadata_backup_path, None)?;
    create_durable_backup(
        &layout.manifest_path(),
        &manifest_backup_path,
        Some(MANIFEST_LIMIT_BYTES),
    )?;
    write_bounded_file(
        &marker_path,
        marker_contents,
        MANIFEST_LIMIT_BYTES,
        correlation_id,
    )?;

    Ok(ProjectUpgrade {
        layout: layout.clone(),
        marker_path,
        metadata_backup_path,
        manifest_backup_path,
        correlation_id: correlation_id.to_owned(),
        committed: false,
    })
}

fn ensure_bounded(contents: &[u8], limit: u64, path: &Path) -> Result<(), FilesystemError> {
    if u64::try_from(contents.len()).unwrap_or(u64::MAX) > limit {
        return Err(FilesystemError::FileTooLarge {
            path: path.to_owned(),
            limit,
        });
    }
    Ok(())
}

fn write_bounded_file(
    path: &Path,
    contents: &[u8],
    limit: u64,
    token: &str,
) -> Result<(), FilesystemError> {
    ensure_bounded(contents, limit, path)?;
    atomic_write(path, contents, token)
}

fn entry_exists_no_follow(path: &Path) -> Result<bool, FilesystemError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(FilesystemError::Io(error)),
    }
}

fn ensure_regular_file(path: &Path, label: &str) -> Result<(), FilesystemError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} must be a regular file"
        )));
    }
    Ok(())
}

fn ensure_regular_directory(path: &Path, label: &str) -> Result<(), FilesystemError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} must be a regular directory"
        )));
    }
    Ok(())
}

fn create_durable_backup(
    source_path: &Path,
    backup_path: &Path,
    limit: Option<u64>,
) -> Result<(), FilesystemError> {
    ensure_regular_file(source_path, "project control file")?;
    let mut source = File::open(source_path)?;
    let source_length = source.metadata()?.len();
    if let Some(limit) = limit {
        if source_length > limit {
            return Err(FilesystemError::FileTooLarge {
                path: source_path.to_owned(),
                limit,
            });
        }
    }

    let mut backup = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(backup_path)?;
    io::copy(&mut source, &mut backup)?;
    backup.flush()?;
    backup.sync_all()?;
    Ok(())
}

fn restore_regular_backup(
    backup_path: &Path,
    destination_path: &Path,
    token: &str,
) -> Result<(), FilesystemError> {
    ensure_regular_file(backup_path, "project recovery backup")?;
    if entry_exists_no_follow(destination_path)? {
        ensure_regular_file(destination_path, "project control file")?;
    }

    let file_name = destination_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| FilesystemError::InvalidPath("file name is not UTF-8".to_owned()))?;
    let temporary_path =
        destination_path.with_file_name(format!(".{file_name}.{token}.restore.tmp"));
    let restore_result = (|| -> Result<(), io::Error> {
        let mut backup = File::open(backup_path)?;
        let mut temporary = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)?;
        io::copy(&mut backup, &mut temporary)?;
        temporary.flush()?;
        temporary.sync_all()?;
        drop(temporary);
        fs::rename(&temporary_path, destination_path)
    })();
    if restore_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    restore_result.map_err(FilesystemError::Io)
}

fn files_equal(left_path: &Path, right_path: &Path) -> Result<bool, FilesystemError> {
    ensure_regular_file(left_path, "project recovery backup")?;
    ensure_regular_file(right_path, "restored project control file")?;
    let mut left = File::open(left_path)?;
    let mut right = File::open(right_path)?;
    if left.metadata()?.len() != right.metadata()?.len() {
        return Ok(false);
    }

    let mut left_buffer = [0_u8; 8192];
    let mut right_buffer = [0_u8; 8192];
    loop {
        let left_read = left.read(&mut left_buffer)?;
        let right_read = right.read(&mut right_buffer)?;
        if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::{
        begin_project_creation, validate_project_layout, FilesystemError, ProjectLayout,
        MANIFEST_LIMIT_BYTES,
    };

    const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000110";
    const CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000111";
    const MARKER: &[u8] = br#"{"stage":"backup_complete"}"#;

    struct ProjectFixture {
        parent: PathBuf,
        layout: ProjectLayout,
    }

    impl ProjectFixture {
        fn layout(&self) -> &ProjectLayout {
            &self.layout
        }
    }

    impl Drop for ProjectFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.parent);
        }
    }

    fn project_fixture(label: &str) -> ProjectFixture {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let parent = std::env::temp_dir().join(format!(
            "teratai-filesystem-upgrade-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&parent).expect("test parent");
        let target = parent.join("fixture.teratai");
        let creation = begin_project_creation(&target, PROJECT_ID, CORRELATION_ID, b"{}")
            .expect("project staging");
        fs::write(creation.metadata_path(), b"original sqlite bytes")
            .expect("metadata placeholder");
        creation
            .write_manifest(br#"{"metadata_schema_version":1}"#)
            .expect("manifest");
        let layout = creation.commit().expect("project commit");
        ProjectFixture { parent, layout }
    }

    fn control_file_bytes(root: &Path) -> (Vec<u8>, Vec<u8>) {
        (
            fs::read(root.join("manifest.json")).expect("manifest bytes"),
            fs::read(root.join("metadata.sqlite")).expect("metadata bytes"),
        )
    }

    #[test]
    fn uncommitted_upgrade_restores_both_control_files() {
        let fixture = project_fixture("restore");
        let layout = fixture.layout();
        let before = control_file_bytes(layout.root());
        {
            let upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
            fs::write(layout.metadata_path(), b"mutated").unwrap();
            upgrade.write_manifest(b"mutated manifest").unwrap();
        }
        assert_eq!(control_file_bytes(layout.root()), before);
    }

    #[test]
    fn committed_upgrade_removes_marker_and_backups() {
        let fixture = project_fixture("commit");
        let layout = fixture.layout();
        let upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
        upgrade.write_manifest(b"new manifest").unwrap();
        upgrade.commit().unwrap();
        assert!(!layout
            .root()
            .join(".project-upgrade-recovery.json")
            .exists());
        assert!(!layout
            .root()
            .join("recovery/metadata-schema-1.sqlite.backup")
            .exists());
        assert!(!layout
            .root()
            .join("recovery/manifest-schema-1.json.backup")
            .exists());
    }

    #[test]
    fn layout_validation_requires_recovery_for_each_upgrade_artifact() {
        for relative_path in [
            ".project-upgrade-recovery.json",
            "recovery/metadata-schema-1.sqlite.backup",
            "recovery/manifest-schema-1.json.backup",
        ] {
            let fixture = project_fixture("recovery-artifact");
            let layout = fixture.layout();
            fs::write(layout.root().join(relative_path), b"recovery proof").unwrap();

            assert!(matches!(
                validate_project_layout(layout.root()),
                Err(FilesystemError::RecoveryRequired(_))
            ));
        }
    }

    #[test]
    fn crash_after_first_backup_is_detected_without_marker() {
        let fixture = project_fixture("backup-before-marker");
        let layout = fixture.layout();
        fs::write(
            layout
                .root()
                .join("recovery/metadata-schema-1.sqlite.backup"),
            b"durable backup",
        )
        .unwrap();

        assert!(matches!(
            validate_project_layout(layout.root()),
            Err(FilesystemError::RecoveryRequired(_))
        ));
    }

    #[test]
    fn invalid_or_oversized_marker_does_not_create_recovery_artifacts() {
        let fixture = project_fixture("invalid-input");
        let layout = fixture.layout();
        let before = control_file_bytes(layout.root());
        let oversized = vec![b'x'; usize::try_from(MANIFEST_LIMIT_BYTES).unwrap() + 1];

        assert!(matches!(
            begin_project_upgrade(layout, "not-a-correlation-id", MARKER),
            Err(FilesystemError::InvalidPath(_))
        ));
        assert!(matches!(
            begin_project_upgrade(layout, CORRELATION_ID, &oversized),
            Err(FilesystemError::FileTooLarge { .. })
        ));
        assert_eq!(control_file_bytes(layout.root()), before);
        assert!(!layout
            .root()
            .join(".project-upgrade-recovery.json")
            .exists());
        assert!(layout
            .root()
            .join("recovery")
            .read_dir()
            .unwrap()
            .next()
            .is_none());
    }

    #[test]
    fn begin_rejects_stale_non_regular_control_and_recovery_entries() {
        let control_fixture = project_fixture("non-regular-control");
        let control_layout = control_fixture.layout();
        fs::remove_file(control_layout.metadata_path()).unwrap();
        fs::create_dir(control_layout.metadata_path()).unwrap();
        assert!(matches!(
            begin_project_upgrade(control_layout, CORRELATION_ID, MARKER),
            Err(FilesystemError::InvalidLayout(_))
        ));

        let recovery_fixture = project_fixture("non-regular-recovery");
        let recovery_layout = recovery_fixture.layout();
        fs::remove_dir(recovery_layout.root().join("recovery")).unwrap();
        fs::write(recovery_layout.root().join("recovery"), b"not a directory").unwrap();
        assert!(matches!(
            begin_project_upgrade(recovery_layout, CORRELATION_ID, MARKER),
            Err(FilesystemError::InvalidLayout(_))
        ));
    }

    #[test]
    fn restore_failure_keeps_marker_and_backups_for_recovery() {
        let fixture = project_fixture("restore-failure");
        let layout = fixture.layout();
        let mut upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
        fs::remove_file(layout.metadata_path()).unwrap();
        fs::create_dir(layout.metadata_path()).unwrap();

        assert!(upgrade.restore().is_err());
        assert!(layout
            .root()
            .join(".project-upgrade-recovery.json")
            .is_file());
        assert!(layout
            .root()
            .join("recovery/metadata-schema-1.sqlite.backup")
            .is_file());
        assert!(layout
            .root()
            .join("recovery/manifest-schema-1.json.backup")
            .is_file());
        std::mem::forget(upgrade);
    }

    #[test]
    fn marker_and_manifest_updates_are_bounded() {
        let fixture = project_fixture("bounded-updates");
        let layout = fixture.layout();
        let upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
        let oversized = vec![b'x'; usize::try_from(MANIFEST_LIMIT_BYTES).unwrap() + 1];

        assert!(matches!(
            upgrade.write_marker(&oversized),
            Err(FilesystemError::FileTooLarge { .. })
        ));
        assert!(matches!(
            upgrade.write_manifest(&oversized),
            Err(FilesystemError::FileTooLarge { .. })
        ));
    }
}
