use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use crate::{
    atomic_write, validate_safe_token, FilesystemError, ProjectLayout, MANIFEST_LIMIT_BYTES,
};

pub(crate) const UPGRADE_MARKER_FILE: &str = ".project-upgrade-recovery.json";
pub(crate) const UPGRADE_LOCK_FILE: &str = ".project-upgrade.lock";
pub(crate) const METADATA_BACKUP_FILE: &str = "metadata-schema-1.sqlite.backup";
pub(crate) const MANIFEST_BACKUP_FILE: &str = "manifest-schema-1.json.backup";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BeginStage {
    BackupsCopied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FinalizeStage {
    CreateCleanupDirectory,
    MoveMetadataBackup,
    AfterMetadataBackupRename,
    MoveManifestBackup,
    AfterManifestBackupRename,
    MoveLock,
    MoveMarker,
    AfterMarkerRename,
    CleanupMetadataBackup,
    CleanupManifestBackup,
    CleanupLock,
    CleanupMarker,
    CleanupDirectory,
}

#[derive(Clone, Copy)]
enum ProofFile {
    MetadataBackup,
    ManifestBackup,
    Marker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    volume: u64,
    file: u64,
    length: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContentProof {
    length: u64,
    digest_a: u64,
    digest_b: u64,
}

/// Bounded content identity for one durable project-upgrade backup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectUpgradeFileProof {
    pub length: u64,
    pub content_digest: String,
}

/// Durable identities for both control-file backups created before a marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectUpgradeBackupProof {
    pub metadata: ProjectUpgradeFileProof,
    pub manifest: ProjectUpgradeFileProof,
}

impl ContentProof {
    fn bounded_file_proof(self) -> ProjectUpgradeFileProof {
        ProjectUpgradeFileProof {
            length: self.length,
            content_digest: format!("proof-v1:{:016x}{:016x}", self.digest_a, self.digest_b),
        }
    }
}

#[derive(Debug)]
struct OwnedFile {
    file: File,
    identity: FileIdentity,
}

/// Owned no-follow metadata handle that pins and verifies one project database.
#[derive(Debug)]
pub struct PinnedProjectMetadata {
    path: PathBuf,
    file: File,
    identity: Arc<Mutex<FileIdentity>>,
}

/// One serialized operation over a shared pinned metadata identity.
///
/// The guard keeps peer stores from observing a committed write before the
/// writer refreshes the authorized mutable file identity.
pub struct PinnedProjectOperation<'a> {
    path: &'a Path,
    file: &'a File,
    identity: MutexGuard<'a, FileIdentity>,
}

type SharedIdentity = Arc<Mutex<FileIdentity>>;
type WeakSharedIdentity = Weak<Mutex<FileIdentity>>;

static PINNED_METADATA_IDENTITIES: OnceLock<Mutex<HashMap<PathBuf, WeakSharedIdentity>>> =
    OnceLock::new();

fn pinned_metadata_identities() -> &'static Mutex<HashMap<PathBuf, WeakSharedIdentity>> {
    PINNED_METADATA_IDENTITIES.get_or_init(|| Mutex::new(HashMap::new()))
}

impl PinnedProjectMetadata {
    #[cfg(any(test, feature = "test-utils"))]
    #[must_use]
    pub fn operation_is_locked_for_test(&self) -> bool {
        matches!(
            self.identity.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        )
    }

    /// Begin one complete read or write operation over this pinned capability.
    ///
    /// # Errors
    ///
    /// Returns an error when the shared operation capability is poisoned.
    pub fn begin_operation(&self) -> Result<PinnedProjectOperation<'_>, FilesystemError> {
        let identity = self.identity.lock().map_err(|_| {
            FilesystemError::InvalidLayout("pinned metadata capability is poisoned".to_owned())
        })?;
        Ok(PinnedProjectOperation {
            path: &self.path,
            file: &self.file,
            identity,
        })
    }

    /// Return the canonical metadata path represented by this capability.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Verify that the owned handle and current path still identify the pinned file.
    ///
    /// # Errors
    ///
    /// Returns an error when the path becomes linked, non-regular, replaced,
    /// or changes identity outside an authorized metadata write.
    pub fn verify(&self) -> Result<(), FilesystemError> {
        self.begin_operation()?.verify()
    }

    /// Accept the current identity after an authorized `SQLite` write completes.
    ///
    /// This is required on Windows because stable `std` exposes mutable file
    /// metadata rather than the full by-handle file ID. The owned handle denies
    /// delete sharing while allowing `SQLite` readers and writers.
    ///
    /// # Errors
    ///
    /// Returns an error when the handle and path no longer identify the same
    /// non-linked regular file.
    pub fn refresh_after_authorized_write(&self) -> Result<(), FilesystemError> {
        self.begin_operation()?.refresh_after_authorized_write()
    }
}

impl PinnedProjectOperation<'_> {
    /// Verify the pinned handle and path against the operation's identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the file was linked, replaced, or changed outside
    /// the serialized authorized operation.
    pub fn verify(&self) -> Result<(), FilesystemError> {
        validate_file_handle(
            self.path,
            self.file,
            *self.identity,
            "pinned project metadata",
        )
    }

    /// Refresh the shared identity after this operation commits a write.
    ///
    /// # Errors
    ///
    /// Returns an error when the handle and path no longer identify the same
    /// non-linked regular file.
    pub fn refresh_after_authorized_write(&mut self) -> Result<(), FilesystemError> {
        let handle_identity =
            identity_from_metadata(&self.file.metadata()?, "pinned project metadata", true)?;
        let path_metadata = fs::symlink_metadata(self.path)?;
        validate_regular_metadata(&path_metadata, "pinned project metadata")?;
        let path_identity =
            identity_from_metadata(&path_metadata, "pinned project metadata", true)?;
        if handle_identity != path_identity {
            return Err(FilesystemError::InvalidLayout(
                "pinned project metadata identity changed".to_owned(),
            ));
        }
        *self.identity = handle_identity;
        Ok(())
    }
}

impl Drop for PinnedProjectMetadata {
    fn drop(&mut self) {
        if let Ok(mut identities) = pinned_metadata_identities().lock() {
            let remove = identities
                .get(&self.path)
                .and_then(Weak::upgrade)
                .is_some_and(|registered| {
                    Arc::ptr_eq(&registered, &self.identity) && Arc::strong_count(&registered) == 2
                });
            if remove {
                identities.remove(&self.path);
            }
        }
    }
}

/// Pin the validated project metadata path with an owned no-follow handle.
///
/// # Errors
///
/// Returns an error when metadata is missing, linked, non-regular, hardlinked
/// where the platform exposes link count, or changes identity while opening.
pub fn pin_project_metadata(
    layout: &ProjectLayout,
) -> Result<PinnedProjectMetadata, FilesystemError> {
    let path = layout.metadata_path();
    let (file, identity) =
        open_verified_regular_with_mode(&path, "project metadata", RegularOpenMode::Pinned)?;
    let identity = shared_pinned_identity(&path, identity)?;
    Ok(PinnedProjectMetadata {
        path,
        file,
        identity,
    })
}

fn shared_pinned_identity(
    path: &Path,
    observed: FileIdentity,
) -> Result<SharedIdentity, FilesystemError> {
    let mut identities = pinned_metadata_identities().lock().map_err(|_| {
        FilesystemError::InvalidLayout("pinned metadata registry is poisoned".to_owned())
    })?;
    if let Some(shared) = identities.get(path).and_then(Weak::upgrade) {
        let authorized = *shared.lock().map_err(|_| {
            FilesystemError::InvalidLayout("pinned metadata capability is poisoned".to_owned())
        })?;
        if observed != authorized {
            return Err(FilesystemError::InvalidLayout(
                "pinned project metadata changed outside an authorized write".to_owned(),
            ));
        }
        return Ok(shared);
    }

    let shared = Arc::new(Mutex::new(observed));
    identities.insert(path.to_owned(), Arc::downgrade(&shared));
    Ok(shared)
}

/// A durable guard for an in-place project metadata upgrade.
#[derive(Debug)]
pub struct ProjectUpgrade {
    layout: ProjectLayout,
    marker_path: PathBuf,
    marker_file: RefCell<OwnedFile>,
    metadata_backup_path: PathBuf,
    manifest_backup_path: PathBuf,
    metadata_backup_file: File,
    manifest_backup_file: File,
    metadata_backup_identity: FileIdentity,
    manifest_backup_identity: FileIdentity,
    metadata_backup_proof: ContentProof,
    manifest_backup_proof: ContentProof,
    lock_path: PathBuf,
    lock_file: Option<File>,
    lock_identity: FileIdentity,
    root_directory: File,
    root_identity: FileIdentity,
    recovery_directory: File,
    recovery_identity: FileIdentity,
    correlation_id: String,
    cleanup_directory: Option<PathBuf>,
    cleanup_directory_file: Option<File>,
    cleanup_identity: Option<FileIdentity>,
    committed: bool,
    finalize_failure: Option<FinalizeStage>,
}

impl ProjectUpgrade {
    /// Atomically replace the bounded recovery marker after a stage change.
    ///
    /// # Errors
    ///
    /// Returns an error when the marker is missing, linked, non-regular, too
    /// large, or cannot be durably replaced.
    pub fn write_marker(&self, contents: &[u8]) -> Result<(), FilesystemError> {
        self.validate_owned_state()?;
        durable_bounded_write(
            &self.marker_path,
            contents,
            MANIFEST_LIMIT_BYTES,
            &self.correlation_id,
        )?;
        let (file, identity) =
            open_verified_regular(&self.marker_path, "upgrade recovery marker", true)?;
        *self.marker_file.borrow_mut() = OwnedFile { file, identity };
        Ok(())
    }

    /// Atomically replace the bounded project manifest.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest is linked, non-regular, too large,
    /// or cannot be durably replaced.
    pub fn write_manifest(&self, contents: &[u8]) -> Result<(), FilesystemError> {
        self.validate_owned_state()?;
        let manifest_path = self.layout.manifest_path();
        let _manifest = open_verified_regular(&manifest_path, "project manifest", false)?;
        durable_bounded_write(
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
        self.validate_owned_state()?;
        self.validate_backup_proofs()?;

        restore_regular_backup(
            &self.metadata_backup_file,
            self.metadata_backup_proof.length,
            &self.layout.metadata_path(),
            &self.correlation_id,
        )?;
        restore_regular_backup(
            &self.manifest_backup_file,
            self.manifest_backup_proof.length,
            &self.layout.manifest_path(),
            &self.correlation_id,
        )?;

        let (metadata, _) =
            open_verified_regular(&self.layout.metadata_path(), "restored metadata", false)?;
        let (manifest, _) =
            open_verified_regular(&self.layout.manifest_path(), "restored manifest", false)?;
        if !files_equal(&self.metadata_backup_file, &metadata)?
            || !files_equal(&self.manifest_backup_file, &manifest)?
        {
            return Err(FilesystemError::InvalidLayout(
                "restored project control files could not be verified".to_owned(),
            ));
        }

        self.finalize_recovery_artifacts()
    }

    /// Commit an already-validated upgrade by clearing its recovery artifacts.
    ///
    /// The marker is removed last so normal project open remains blocked while
    /// either backup cleanup is still in progress.
    ///
    /// # Errors
    ///
    /// Returns an error when a control/recovery entry is unsafe or an artifact
    /// cannot be removed. The caller retains this guard to observe and prove
    /// any required restoration before the error crosses a trust boundary.
    pub fn commit(&mut self) -> Result<(), FilesystemError> {
        self.validate_owned_state()?;
        self.validate_backup_proofs()?;
        let _metadata =
            open_verified_regular(&self.layout.metadata_path(), "project metadata", true)?;
        let _manifest =
            open_verified_regular(&self.layout.manifest_path(), "project manifest", true)?;
        self.finalize_recovery_artifacts()
    }

    fn validate_owned_state(&self) -> Result<(), FilesystemError> {
        validate_directory_handle(
            self.layout.root(),
            &self.root_directory,
            self.root_identity,
            "project root",
        )?;
        validate_directory_handle(
            &self.layout.root().join("recovery"),
            &self.recovery_directory,
            self.recovery_identity,
            "recovery directory",
        )?;
        let lock_file = self.lock_file.as_ref().ok_or_else(|| {
            FilesystemError::InvalidLayout("project upgrade lock ownership was lost".to_owned())
        })?;
        validate_file_handle(
            &self.lock_path,
            lock_file,
            self.lock_identity,
            "project upgrade lock",
        )?;
        let marker = self.marker_file.borrow();
        validate_file_handle(
            &self.marker_path,
            &marker.file,
            marker.identity,
            "upgrade recovery marker",
        )?;
        self.validate_cleanup_directory()?;
        Ok(())
    }

    fn validate_backup_proofs(&self) -> Result<(), FilesystemError> {
        validate_file_handle(
            &self.metadata_backup_path,
            &self.metadata_backup_file,
            self.metadata_backup_identity,
            "metadata recovery backup",
        )?;
        validate_file_handle(
            &self.manifest_backup_path,
            &self.manifest_backup_file,
            self.manifest_backup_identity,
            "manifest recovery backup",
        )?;
        if content_proof(&self.metadata_backup_file)? != self.metadata_backup_proof
            || content_proof(&self.manifest_backup_file)? != self.manifest_backup_proof
        {
            return Err(FilesystemError::InvalidLayout(
                "project recovery backup content changed".to_owned(),
            ));
        }
        Ok(())
    }

    fn finalize_recovery_artifacts(&mut self) -> Result<(), FilesystemError> {
        let cleanup_directory = self.prepare_cleanup_directory()?;

        self.move_proof_file(
            FinalizeStage::MoveMetadataBackup,
            ProofFile::MetadataBackup,
            &cleanup_directory.join(METADATA_BACKUP_FILE),
        )?;
        self.move_proof_file(
            FinalizeStage::MoveManifestBackup,
            ProofFile::ManifestBackup,
            &cleanup_directory.join(MANIFEST_BACKUP_FILE),
        )?;
        self.move_proof_file(
            FinalizeStage::MoveMarker,
            ProofFile::Marker,
            &cleanup_directory.join(UPGRADE_MARKER_FILE),
        )?;

        // The marker move plus directory sync is the commit point. The owned
        // lock remains at its contract path until after this point, preventing
        // a cooperating owner from entering during a failed marker sync.
        // Windows std does not expose MOVEFILE_WRITE_THROUGH; the safe
        // best-available sequence is synced file contents, atomic rename, then
        // directory-handle sync_all.
        self.committed = true;
        if !self.skip_cleanup_stage(FinalizeStage::MoveLock) {
            let lock_destination = cleanup_directory.join(UPGRADE_LOCK_FILE);
            if fs::rename(&self.lock_path, &lock_destination).is_ok() {
                self.lock_path = lock_destination;
                let _ = sync_directory(&cleanup_directory);
                let _ = sync_directory(&self.layout.root().join("recovery"));
            }
        }
        drop(self.lock_file.take());
        let cleanup = [
            (
                FinalizeStage::CleanupMetadataBackup,
                Some(self.metadata_backup_path.clone()),
            ),
            (
                FinalizeStage::CleanupManifestBackup,
                Some(self.manifest_backup_path.clone()),
            ),
            (
                FinalizeStage::CleanupLock,
                (self.lock_path.parent() == Some(cleanup_directory.as_path()))
                    .then(|| self.lock_path.clone()),
            ),
            (FinalizeStage::CleanupMarker, Some(self.marker_path.clone())),
        ];
        for (stage, path) in cleanup {
            if !self.skip_cleanup_stage(stage) {
                if let Some(path) = path {
                    let _ = fs::remove_file(&path);
                    let _ = sync_directory(&cleanup_directory);
                }
            }
        }
        if !self.skip_cleanup_stage(FinalizeStage::CleanupDirectory) {
            drop(self.cleanup_directory_file.take());
            self.cleanup_identity = None;
            let _ = fs::remove_dir(&cleanup_directory);
            let _ = sync_directory(self.layout.root());
        }
        Ok(())
    }

    fn prepare_cleanup_directory(&mut self) -> Result<PathBuf, FilesystemError> {
        if let Some(path) = &self.cleanup_directory {
            self.validate_cleanup_directory()?;
            return Ok(path.clone());
        }
        self.fail_finalize_stage(FinalizeStage::CreateCleanupDirectory)?;
        let path = self
            .layout
            .root()
            .join(format!(".project-upgrade-cleanup-{}", self.correlation_id));
        fs::create_dir(&path)?;
        self.cleanup_directory = Some(path.clone());
        let (directory, identity) = open_verified_directory(&path, "upgrade cleanup directory")?;
        self.cleanup_directory_file = Some(directory);
        self.cleanup_identity = Some(identity);
        sync_directory(self.layout.root())?;
        Ok(path)
    }

    fn validate_cleanup_directory(&self) -> Result<(), FilesystemError> {
        match (
            &self.cleanup_directory,
            &self.cleanup_directory_file,
            self.cleanup_identity,
        ) {
            (Some(path), Some(directory), Some(identity)) => {
                validate_directory_handle(path, directory, identity, "upgrade cleanup directory")
            }
            (None, None, None) => Ok(()),
            _ => Err(FilesystemError::InvalidLayout(
                "upgrade cleanup directory ownership is incomplete".to_owned(),
            )),
        }
    }

    #[cfg(test)]
    fn prepare_cleanup_directory_for_test(&mut self) -> Result<PathBuf, FilesystemError> {
        self.prepare_cleanup_directory()
    }

    fn move_proof_file(
        &mut self,
        stage: FinalizeStage,
        proof: ProofFile,
        destination: &Path,
    ) -> Result<(), FilesystemError> {
        let source = match proof {
            ProofFile::MetadataBackup => self.metadata_backup_path.clone(),
            ProofFile::ManifestBackup => self.manifest_backup_path.clone(),
            ProofFile::Marker => self.marker_path.clone(),
        };
        if source == destination {
            self.validate_moved_proof(proof, destination)?;
            return Ok(());
        }
        self.validate_cleanup_directory()?;
        self.validate_proof_parent(&source)?;
        self.validate_moved_proof(proof, &source)?;
        if entry_exists_no_follow(destination)? {
            return Err(FilesystemError::RecoveryRequired(destination.to_owned()));
        }
        self.fail_finalize_stage(stage)?;
        fs::rename(&source, destination)?;
        let destination_owned = destination.to_owned();
        match proof {
            ProofFile::MetadataBackup => self.metadata_backup_path = destination_owned,
            ProofFile::ManifestBackup => self.manifest_backup_path = destination_owned,
            ProofFile::Marker => self.marker_path = destination_owned,
        }
        let after_rename_stage = match proof {
            ProofFile::MetadataBackup => FinalizeStage::AfterMetadataBackupRename,
            ProofFile::ManifestBackup => FinalizeStage::AfterManifestBackupRename,
            ProofFile::Marker => FinalizeStage::AfterMarkerRename,
        };
        self.fail_finalize_stage(after_rename_stage)?;
        if entry_exists_no_follow(&source)? {
            return Err(FilesystemError::InvalidLayout(
                "recovery proof source remained after rename".to_owned(),
            ));
        }
        self.validate_moved_proof(proof, destination)?;
        self.validate_cleanup_directory()?;
        self.validate_proof_parent(&source)?;
        sync_directory(destination.parent().ok_or_else(|| {
            FilesystemError::InvalidPath("cleanup artifact parent is missing".to_owned())
        })?)?;
        sync_directory(source.parent().ok_or_else(|| {
            FilesystemError::InvalidPath("recovery artifact parent is missing".to_owned())
        })?)
    }

    fn validate_moved_proof(&self, proof: ProofFile, path: &Path) -> Result<(), FilesystemError> {
        match proof {
            ProofFile::MetadataBackup => validate_file_handle(
                path,
                &self.metadata_backup_file,
                self.metadata_backup_identity,
                "metadata recovery backup",
            ),
            ProofFile::ManifestBackup => validate_file_handle(
                path,
                &self.manifest_backup_file,
                self.manifest_backup_identity,
                "manifest recovery backup",
            ),
            ProofFile::Marker => {
                let marker = self.marker_file.borrow();
                validate_file_handle(
                    path,
                    &marker.file,
                    marker.identity,
                    "upgrade recovery marker",
                )
            }
        }
    }

    fn validate_proof_parent(&self, source: &Path) -> Result<(), FilesystemError> {
        let parent = source.parent().ok_or_else(|| {
            FilesystemError::InvalidPath("recovery proof parent is missing".to_owned())
        })?;
        let recovery_path = self.layout.root().join("recovery");
        if parent == self.layout.root() {
            return validate_directory_handle(
                self.layout.root(),
                &self.root_directory,
                self.root_identity,
                "project root",
            );
        }
        if parent == recovery_path {
            return validate_directory_handle(
                &recovery_path,
                &self.recovery_directory,
                self.recovery_identity,
                "recovery directory",
            );
        }
        if self.cleanup_directory.as_deref() == Some(parent) {
            return self.validate_cleanup_directory();
        }
        Err(FilesystemError::InvalidLayout(
            "recovery proof parent is not owned by this guard".to_owned(),
        ))
    }

    #[cfg(test)]
    fn inject_finalize_failure(&mut self, stage: FinalizeStage) {
        self.finalize_failure = Some(stage);
    }

    fn fail_finalize_stage(&mut self, stage: FinalizeStage) -> Result<(), FilesystemError> {
        if self.finalize_failure == Some(stage) {
            self.finalize_failure = None;
            return Err(FilesystemError::Io(io::Error::other(
                "injected finalize failure",
            )));
        }
        Ok(())
    }

    fn skip_cleanup_stage(&mut self, stage: FinalizeStage) -> bool {
        if self.finalize_failure == Some(stage) {
            self.finalize_failure = None;
            return true;
        }
        false
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
pub fn begin_project_upgrade<E, F>(
    layout: &ProjectLayout,
    correlation_id: &str,
    marker_builder: F,
) -> Result<ProjectUpgrade, E>
where
    E: From<FilesystemError>,
    F: FnOnce(&ProjectUpgradeBackupProof) -> Result<Vec<u8>, E>,
{
    begin_project_upgrade_inner(layout, correlation_id, marker_builder, |_, _| {})
}

#[cfg(test)]
fn begin_project_upgrade_observed<F>(
    layout: &ProjectLayout,
    correlation_id: &str,
    marker_contents: &[u8],
    observer: F,
) -> Result<ProjectUpgrade, FilesystemError>
where
    F: FnMut(BeginStage, &ProjectLayout),
{
    begin_project_upgrade_inner(
        layout,
        correlation_id,
        |_| Ok(marker_contents.to_vec()),
        observer,
    )
}

fn begin_project_upgrade_inner<E, F, O>(
    layout: &ProjectLayout,
    correlation_id: &str,
    marker_builder: F,
    mut observer: O,
) -> Result<ProjectUpgrade, E>
where
    E: From<FilesystemError>,
    F: FnOnce(&ProjectUpgradeBackupProof) -> Result<Vec<u8>, E>,
    O: FnMut(BeginStage, &ProjectLayout),
{
    validate_safe_token(correlation_id, "correlation_id")?;
    let marker_path = layout.root().join(UPGRADE_MARKER_FILE);

    let recovery_directory = layout.root().join("recovery");
    let (root_directory, root_identity) = open_verified_directory(layout.root(), "project root")?;
    let (recovery_directory_file, recovery_identity) =
        open_verified_directory(&recovery_directory, "recovery directory")?;
    let (metadata_source, metadata_source_identity) =
        open_verified_regular(&layout.metadata_path(), "project metadata", false)?;
    let (manifest_source, manifest_source_identity) =
        open_verified_regular(&layout.manifest_path(), "project manifest", false)?;
    if manifest_source_identity.length > MANIFEST_LIMIT_BYTES {
        return Err(FilesystemError::FileTooLarge {
            path: layout.manifest_path(),
            limit: MANIFEST_LIMIT_BYTES,
        }
        .into());
    }

    let metadata_backup_path = recovery_directory.join(METADATA_BACKUP_FILE);
    let manifest_backup_path = recovery_directory.join(MANIFEST_BACKUP_FILE);
    let lock_path = recovery_directory.join(UPGRADE_LOCK_FILE);
    for artifact in [
        &marker_path,
        &metadata_backup_path,
        &manifest_backup_path,
        &lock_path,
    ] {
        if entry_exists_no_follow(artifact)? {
            return Err(FilesystemError::RecoveryRequired(artifact.clone()).into());
        }
    }

    let (lock_file, lock_identity) = create_owned_file(&lock_path, correlation_id.as_bytes())?;
    sync_directory(&recovery_directory)?;
    let (metadata_backup_file, metadata_backup_identity, metadata_backup_proof) =
        create_durable_backup(
            &metadata_source,
            metadata_source_identity,
            &metadata_backup_path,
        )?;
    sync_directory(&recovery_directory)?;
    let (manifest_backup_file, manifest_backup_identity, manifest_backup_proof) =
        create_durable_backup(
            &manifest_source,
            manifest_source_identity,
            &manifest_backup_path,
        )?;
    sync_directory(&recovery_directory)?;

    observer(BeginStage::BackupsCopied, layout);
    validate_source_snapshot(
        &layout.metadata_path(),
        &metadata_source,
        metadata_source_identity,
        &metadata_backup_file,
        "project metadata",
    )?;
    validate_source_snapshot(
        &layout.manifest_path(),
        &manifest_source,
        manifest_source_identity,
        &manifest_backup_file,
        "project manifest",
    )?;
    let (marker_file, marker_identity) = publish_initial_marker(
        &marker_path,
        metadata_backup_proof,
        manifest_backup_proof,
        correlation_id,
        marker_builder,
    )?;

    Ok(ProjectUpgrade {
        layout: layout.clone(),
        marker_path,
        marker_file: RefCell::new(OwnedFile {
            file: marker_file,
            identity: marker_identity,
        }),
        metadata_backup_path,
        manifest_backup_path,
        metadata_backup_file,
        manifest_backup_file,
        metadata_backup_identity,
        manifest_backup_identity,
        metadata_backup_proof,
        manifest_backup_proof,
        lock_path,
        lock_file: Some(lock_file),
        lock_identity,
        root_directory,
        root_identity,
        recovery_directory: recovery_directory_file,
        recovery_identity,
        correlation_id: correlation_id.to_owned(),
        cleanup_directory: None,
        cleanup_directory_file: None,
        cleanup_identity: None,
        committed: false,
        finalize_failure: None,
    })
}

fn publish_initial_marker<E, F>(
    marker_path: &Path,
    metadata_backup_proof: ContentProof,
    manifest_backup_proof: ContentProof,
    correlation_id: &str,
    marker_builder: F,
) -> Result<(File, FileIdentity), E>
where
    E: From<FilesystemError>,
    F: FnOnce(&ProjectUpgradeBackupProof) -> Result<Vec<u8>, E>,
{
    let backup_proof = ProjectUpgradeBackupProof {
        metadata: metadata_backup_proof.bounded_file_proof(),
        manifest: manifest_backup_proof.bounded_file_proof(),
    };
    let marker_contents = marker_builder(&backup_proof)?;
    ensure_bounded(&marker_contents, MANIFEST_LIMIT_BYTES, marker_path)?;
    durable_bounded_write(
        marker_path,
        &marker_contents,
        MANIFEST_LIMIT_BYTES,
        correlation_id,
    )?;
    open_verified_regular(marker_path, "upgrade recovery marker", true).map_err(E::from)
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

fn durable_bounded_write(
    path: &Path,
    contents: &[u8],
    limit: u64,
    token: &str,
) -> Result<(), FilesystemError> {
    ensure_bounded(contents, limit, path)?;
    atomic_write(path, contents, token)?;
    sync_directory(
        path.parent().ok_or_else(|| {
            FilesystemError::InvalidPath("control file parent is missing".to_owned())
        })?,
    )
}

fn entry_exists_no_follow(path: &Path) -> Result<bool, FilesystemError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(FilesystemError::Io(error)),
    }
}

fn open_verified_regular(
    path: &Path,
    label: &str,
    exclusive: bool,
) -> Result<(File, FileIdentity), FilesystemError> {
    let mode = if exclusive {
        RegularOpenMode::Exclusive
    } else {
        RegularOpenMode::Shared
    };
    open_verified_regular_with_mode(path, label, mode)
}

#[derive(Clone, Copy)]
enum RegularOpenMode {
    Shared,
    Exclusive,
    Pinned,
}

fn open_verified_regular_with_mode(
    path: &Path,
    label: &str,
    mode: RegularOpenMode,
) -> Result<(File, FileIdentity), FilesystemError> {
    let before = fs::symlink_metadata(path)?;
    validate_regular_metadata(&before, label)?;
    let mut options = OpenOptions::new();
    options.read(true);
    match mode {
        RegularOpenMode::Shared => {}
        RegularOpenMode::Exclusive => configure_exclusive_file_open(&mut options),
        RegularOpenMode::Pinned => configure_pinned_file_open(&mut options),
    }
    let file = options.open(path)?;
    let handle_identity = identity_from_metadata(&file.metadata()?, label, true)?;
    let after = fs::symlink_metadata(path)?;
    validate_regular_metadata(&after, label)?;
    let path_identity = identity_from_metadata(&after, label, true)?;
    if handle_identity != path_identity {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} identity changed while opening"
        )));
    }
    Ok((file, handle_identity))
}

fn open_verified_directory(
    path: &Path,
    label: &str,
) -> Result<(File, FileIdentity), FilesystemError> {
    let before = fs::symlink_metadata(path)?;
    validate_directory_metadata(&before, label)?;
    let mut options = OpenOptions::new();
    options.read(true);
    configure_owned_directory_open(&mut options);
    let directory = options.open(path)?;
    let handle_identity = identity_from_metadata(&directory.metadata()?, label, false)?;
    let after = fs::symlink_metadata(path)?;
    validate_directory_metadata(&after, label)?;
    let path_identity = identity_from_metadata(&after, label, false)?;
    if handle_identity != path_identity {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} identity changed while opening"
        )));
    }
    Ok((directory, handle_identity))
}

fn create_durable_backup(
    source: &File,
    source_identity: FileIdentity,
    backup_path: &Path,
) -> Result<(File, FileIdentity, ContentProof), FilesystemError> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    configure_exclusive_file_open(&mut options);
    let mut backup = options.open(backup_path)?;
    let mut source_reader = source.try_clone()?;
    source_reader.seek(SeekFrom::Start(0))?;
    copy_exact_snapshot(&mut source_reader, &mut backup, source_identity.length)?;
    backup.flush()?;
    backup.sync_all()?;
    let identity = identity_from_metadata(&backup.metadata()?, "project recovery backup", true)?;
    let path_identity = identity_from_metadata(
        &fs::symlink_metadata(backup_path)?,
        "project recovery backup",
        true,
    )?;
    if identity != path_identity {
        return Err(FilesystemError::InvalidLayout(
            "project recovery backup identity changed".to_owned(),
        ));
    }
    let proof = content_proof(&backup)?;
    Ok((backup, identity, proof))
}

fn restore_regular_backup(
    backup: &File,
    backup_length: u64,
    destination_path: &Path,
    token: &str,
) -> Result<(), FilesystemError> {
    let _destination = if entry_exists_no_follow(destination_path)? {
        Some(open_verified_regular(
            destination_path,
            "project control file",
            true,
        )?)
    } else {
        None
    };

    let file_name = destination_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| FilesystemError::InvalidPath("file name is not UTF-8".to_owned()))?;
    let temporary_path =
        destination_path.with_file_name(format!(".{file_name}.{token}.restore.tmp"));
    let restore_result = (|| -> Result<(), FilesystemError> {
        let mut backup_reader = backup.try_clone()?;
        backup_reader.seek(SeekFrom::Start(0))?;
        let mut options = OpenOptions::new();
        options.create_new(true).read(true).write(true);
        configure_exclusive_file_open(&mut options);
        let mut temporary = options.open(&temporary_path)?;
        copy_exact_snapshot(&mut backup_reader, &mut temporary, backup_length)?;
        temporary.flush()?;
        temporary.sync_all()?;
        drop(temporary);
        fs::rename(&temporary_path, destination_path)?;
        sync_directory(destination_path.parent().ok_or_else(|| {
            FilesystemError::InvalidPath("control file parent is missing".to_owned())
        })?)
    })();
    if restore_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    restore_result
}

fn files_equal(left: &File, right: &File) -> Result<bool, FilesystemError> {
    let mut left = left.try_clone()?;
    let mut right = right.try_clone()?;
    left.seek(SeekFrom::Start(0))?;
    right.seek(SeekFrom::Start(0))?;
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

fn copy_exact_snapshot<R: Read, W: Write>(
    source: &mut R,
    destination: &mut W,
    expected_length: u64,
) -> Result<(), FilesystemError> {
    let mut remaining = expected_length;
    let mut buffer = [0_u8; 8192];
    while remaining > 0 {
        let bounded = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = source.read(&mut buffer[..bounded])?;
        if read == 0 {
            return Err(FilesystemError::InvalidLayout(
                "project control file shrank while being backed up".to_owned(),
            ));
        }
        destination.write_all(&buffer[..read])?;
        remaining -= u64::try_from(read).unwrap_or(u64::MAX);
    }
    let mut growth = [0_u8; 1];
    if source.read(&mut growth)? != 0 {
        return Err(FilesystemError::InvalidLayout(
            "project control file grew while being backed up".to_owned(),
        ));
    }
    Ok(())
}

fn validate_source_snapshot(
    path: &Path,
    source: &File,
    expected_identity: FileIdentity,
    backup: &File,
    label: &str,
) -> Result<(), FilesystemError> {
    validate_file_handle(path, source, expected_identity, label)?;
    if !files_equal(source, backup)? {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} content changed while recovery proof was created"
        )));
    }
    Ok(())
}

fn validate_file_handle(
    path: &Path,
    file: &File,
    expected: FileIdentity,
    label: &str,
) -> Result<(), FilesystemError> {
    let current = identity_from_metadata(&file.metadata()?, label, true)?;
    let path_metadata = fs::symlink_metadata(path)?;
    validate_regular_metadata(&path_metadata, label)?;
    let path_identity = identity_from_metadata(&path_metadata, label, true)?;
    if current != expected || path_identity != expected {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} identity changed"
        )));
    }
    Ok(())
}

fn validate_directory_handle(
    path: &Path,
    directory: &File,
    expected: FileIdentity,
    label: &str,
) -> Result<(), FilesystemError> {
    let current = identity_from_metadata(&directory.metadata()?, label, false)?;
    let path_metadata = fs::symlink_metadata(path)?;
    validate_directory_metadata(&path_metadata, label)?;
    let path_identity = identity_from_metadata(&path_metadata, label, false)?;
    if current != expected || path_identity != expected {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} identity changed"
        )));
    }
    Ok(())
}

fn create_owned_file(
    path: &Path,
    contents: &[u8],
) -> Result<(File, FileIdentity), FilesystemError> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    configure_exclusive_file_open(&mut options);
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.flush()?;
    file.sync_all()?;
    let identity = identity_from_metadata(&file.metadata()?, "project upgrade lock", true)?;
    Ok((file, identity))
}

fn content_proof(file: &File) -> Result<ContentProof, FilesystemError> {
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;
    let mut digest_a = 0xcbf2_9ce4_8422_2325_u64;
    let mut digest_b = 0x9e37_79b9_7f4a_7c15_u64;
    let mut length = 0_u64;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        length = length.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        for byte in &buffer[..read] {
            digest_a ^= u64::from(*byte);
            digest_a = digest_a.wrapping_mul(0x0000_0100_0000_01b3);
            digest_b = digest_b.rotate_left(7) ^ u64::from(*byte);
            digest_b = digest_b.wrapping_mul(0x9e37_79b1_85eb_ca87);
        }
    }
    Ok(ContentProof {
        length,
        digest_a,
        digest_b,
    })
}

fn sync_directory(path: &Path) -> Result<(), FilesystemError> {
    let metadata = fs::symlink_metadata(path)?;
    validate_directory_metadata(&metadata, "filesystem namespace directory")?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    configure_sync_directory_open(&mut options);
    options.open(path)?.sync_all()?;
    Ok(())
}

fn validate_regular_metadata(metadata: &fs::Metadata, label: &str) -> Result<(), FilesystemError> {
    if metadata.file_type().is_symlink() || !metadata.is_file() || is_reparse_point(metadata) {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} must be a non-linked regular file"
        )));
    }
    let _ = identity_from_metadata(metadata, label, true)?;
    Ok(())
}

fn validate_directory_metadata(
    metadata: &fs::Metadata,
    label: &str,
) -> Result<(), FilesystemError> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() || is_reparse_point(metadata) {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} must be a non-linked directory"
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn identity_from_metadata(
    metadata: &fs::Metadata,
    label: &str,
    require_single_link: bool,
) -> Result<FileIdentity, FilesystemError> {
    use std::os::windows::fs::MetadataExt;

    if is_reparse_point(metadata) {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} must not be a reparse point"
        )));
    }
    // Stable std exposes timestamps/size and reparse attributes, but Windows
    // file ID and hardlink count remain behind `windows_by_handle`. The held
    // handles plus content proof close cooperating mutation races; full Windows
    // hardlink rejection requires an audited OS API boundary outside this crate.
    Ok(FileIdentity {
        volume: metadata.creation_time(),
        file: if require_single_link {
            metadata.last_write_time()
        } else {
            0
        },
        length: if require_single_link {
            metadata.file_size()
        } else {
            0
        },
    })
}

#[cfg(unix)]
fn identity_from_metadata(
    metadata: &fs::Metadata,
    label: &str,
    require_single_link: bool,
) -> Result<FileIdentity, FilesystemError> {
    use std::os::unix::fs::MetadataExt;

    if require_single_link && metadata.nlink() != 1 {
        return Err(FilesystemError::InvalidLayout(format!(
            "{label} must not have hardlinks"
        )));
    }
    Ok(FileIdentity {
        volume: metadata.dev(),
        file: metadata.ino(),
        length: metadata.size(),
    })
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn configure_exclusive_file_open(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    options.share_mode(FILE_SHARE_DELETE);
}

#[cfg(not(windows))]
fn configure_exclusive_file_open(_options: &mut OpenOptions) {}

#[cfg(windows)]
fn configure_pinned_file_open(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
}

#[cfg(not(windows))]
fn configure_pinned_file_open(_options: &mut OpenOptions) {}

#[cfg(windows)]
fn configure_owned_directory_open(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    options
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
}

#[cfg(not(windows))]
fn configure_owned_directory_open(_options: &mut OpenOptions) {}

#[cfg(windows)]
fn configure_sync_directory_open(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    options
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
}

#[cfg(not(windows))]
fn configure_sync_directory_open(_options: &mut OpenOptions) {}

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

    fn begin_project_upgrade(
        layout: &ProjectLayout,
        correlation_id: &str,
        marker_contents: &[u8],
    ) -> Result<ProjectUpgrade, FilesystemError> {
        super::begin_project_upgrade(layout, correlation_id, |_| Ok(marker_contents.to_vec()))
    }

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
        let mut upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
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
            "recovery/.project-upgrade.lock",
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
    fn invalid_input_is_rejected_before_backup_and_oversized_built_marker_fails_closed() {
        let fixture = project_fixture("invalid-input");
        let layout = fixture.layout();
        let before = control_file_bytes(layout.root());

        assert!(matches!(
            begin_project_upgrade(layout, "not-a-correlation-id", MARKER),
            Err(FilesystemError::InvalidPath(_))
        ));
        assert_eq!(control_file_bytes(layout.root()), before);
        assert!(layout
            .root()
            .join("recovery")
            .read_dir()
            .unwrap()
            .next()
            .is_none());

        let oversized_fixture = project_fixture("oversized-built-marker");
        let oversized_layout = oversized_fixture.layout();
        let oversized_before = control_file_bytes(oversized_layout.root());
        let oversized = vec![b'x'; usize::try_from(MANIFEST_LIMIT_BYTES).unwrap() + 1];
        assert!(matches!(
            begin_project_upgrade(oversized_layout, CORRELATION_ID, &oversized),
            Err(FilesystemError::FileTooLarge { .. })
        ));
        assert_eq!(
            control_file_bytes(oversized_layout.root()),
            oversized_before
        );
        assert!(!oversized_layout
            .root()
            .join(".project-upgrade-recovery.json")
            .exists());
        assert!(matches!(
            validate_project_layout(oversized_layout.root()),
            Err(FilesystemError::RecoveryRequired(_))
        ));
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

    #[test]
    fn finalize_failures_before_marker_move_restore_original_controls() {
        for stage in [
            FinalizeStage::CreateCleanupDirectory,
            FinalizeStage::MoveMetadataBackup,
            FinalizeStage::MoveManifestBackup,
            FinalizeStage::MoveMarker,
        ] {
            let fixture = project_fixture("finalize-before-marker");
            let layout = fixture.layout();
            let before = control_file_bytes(layout.root());
            let mut upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
            fs::write(layout.metadata_path(), b"mutated metadata").unwrap();
            upgrade.write_manifest(b"mutated manifest").unwrap();
            upgrade.inject_finalize_failure(stage);

            assert!(upgrade.commit().is_err(), "stage {stage:?}");
            upgrade
                .restore()
                .expect("restore after failed pre-commit finalization");
            assert_eq!(control_file_bytes(layout.root()), before, "stage {stage:?}");
            validate_project_layout(layout.root()).expect("restored layout");
        }
    }

    #[test]
    fn partial_move_faults_before_directory_sync_restore_original_controls() {
        for stage in [
            FinalizeStage::AfterMetadataBackupRename,
            FinalizeStage::AfterManifestBackupRename,
            FinalizeStage::AfterMarkerRename,
        ] {
            let fixture = project_fixture("partial-proof-move");
            let layout = fixture.layout();
            let before = control_file_bytes(layout.root());
            let mut upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
            fs::write(layout.metadata_path(), b"mutated metadata").unwrap();
            upgrade.write_manifest(b"mutated manifest").unwrap();
            upgrade.inject_finalize_failure(stage);

            assert!(upgrade.commit().is_err(), "stage {stage:?}");
            upgrade
                .restore()
                .expect("restore after failed partial proof finalization");
            assert_eq!(control_file_bytes(layout.root()), before, "stage {stage:?}");
            validate_project_layout(layout.root()).expect("restored layout");
        }
    }

    #[test]
    fn cleanup_failures_after_marker_move_do_not_report_failed_commit() {
        for stage in [
            FinalizeStage::MoveLock,
            FinalizeStage::CleanupMetadataBackup,
            FinalizeStage::CleanupManifestBackup,
            FinalizeStage::CleanupLock,
            FinalizeStage::CleanupMarker,
            FinalizeStage::CleanupDirectory,
        ] {
            let fixture = project_fixture("finalize-after-marker");
            let layout = fixture.layout();
            let mut upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
            upgrade.write_manifest(b"committed manifest").unwrap();
            upgrade.inject_finalize_failure(stage);

            upgrade.commit().expect("logical commit after marker move");
            if stage == FinalizeStage::MoveLock {
                assert!(matches!(
                    validate_project_layout(layout.root()),
                    Err(FilesystemError::RecoveryRequired(_))
                ));
            } else {
                validate_project_layout(layout.root()).expect("committed layout");
            }
            assert_eq!(
                fs::read(layout.manifest_path()).unwrap(),
                b"committed manifest",
                "stage {stage:?}"
            );
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
            assert_eq!(
                layout
                    .root()
                    .join("recovery/.project-upgrade.lock")
                    .exists(),
                stage == FinalizeStage::MoveLock
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn begin_rejects_hardlinked_control_file() {
        let fixture = project_fixture("hardlink-control");
        let layout = fixture.layout();
        fs::hard_link(
            layout.metadata_path(),
            layout.root().join("metadata-hardlink.sqlite"),
        )
        .unwrap();

        assert!(matches!(
            begin_project_upgrade(layout, CORRELATION_ID, MARKER),
            Err(FilesystemError::InvalidLayout(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn begin_rejects_reparse_recovery_directory() {
        use std::os::windows::fs::symlink_dir;

        let fixture = project_fixture("reparse-recovery");
        let layout = fixture.layout();
        let recovery_path = layout.root().join("recovery");
        let actual_path = layout.root().join("actual-recovery");
        fs::remove_dir(&recovery_path).unwrap();
        fs::create_dir(&actual_path).unwrap();
        if let Err(error) = symlink_dir(&actual_path, &recovery_path) {
            if error.kind() == io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("recovery reparse point: {error}");
        }

        assert!(matches!(
            begin_project_upgrade(layout, CORRELATION_ID, MARKER),
            Err(FilesystemError::InvalidLayout(_))
        ));
    }

    #[test]
    fn bounded_copy_rejects_growth_without_writing_past_snapshot() {
        let bytes = b"originalgrowth";
        let mut source = io::Cursor::new(bytes);
        let mut destination = Vec::new();

        assert!(matches!(
            copy_exact_snapshot(
                &mut source,
                &mut destination,
                u64::try_from(b"original".len()).unwrap(),
            ),
            Err(FilesystemError::InvalidLayout(_))
        ));
        assert_eq!(destination, b"original");
    }

    #[test]
    fn concurrent_mutation_before_marker_keeps_original_backup_and_blocks_open() {
        let fixture = project_fixture("concurrent-mutation");
        let layout = fixture.layout();
        let original_metadata = fs::read(layout.metadata_path()).unwrap();

        let result = begin_project_upgrade_observed(
            layout,
            CORRELATION_ID,
            MARKER,
            |stage, observed_layout| {
                if stage == BeginStage::BackupsCopied {
                    fs::write(observed_layout.metadata_path(), b"concurrent mutation").unwrap();
                }
            },
        );

        assert!(matches!(result, Err(FilesystemError::InvalidLayout(_))));
        assert_eq!(
            fs::read(
                layout
                    .root()
                    .join("recovery/metadata-schema-1.sqlite.backup")
            )
            .unwrap(),
            original_metadata
        );
        assert!(matches!(
            validate_project_layout(layout.root()),
            Err(FilesystemError::RecoveryRequired(_))
        ));
    }

    #[test]
    fn upgrade_lock_prevents_a_second_owner() {
        let fixture = project_fixture("exclusive-owner");
        let layout = fixture.layout();
        let upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();

        assert!(layout
            .root()
            .join("recovery/.project-upgrade.lock")
            .is_file());
        assert!(matches!(
            begin_project_upgrade(layout, CORRELATION_ID, MARKER),
            Err(FilesystemError::RecoveryRequired(_))
        ));
        drop(upgrade);
    }

    #[cfg(windows)]
    #[test]
    fn marker_handle_remains_exclusively_owned_until_finalization() {
        use std::os::windows::fs::OpenOptionsExt;

        let fixture = project_fixture("owned-marker");
        let layout = fixture.layout();
        let upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
        let marker_path = layout.root().join(".project-upgrade-recovery.json");

        assert!(OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0x0000_0001 | 0x0000_0002 | 0x0000_0004)
            .open(marker_path)
            .is_err());
        drop(upgrade);
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_directory_cannot_be_swapped_while_guard_owns_it() {
        let fixture = project_fixture("owned-cleanup-directory");
        let layout = fixture.layout();
        let mut upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
        let cleanup = upgrade.prepare_cleanup_directory_for_test().unwrap();
        let swapped = layout.root().join("swapped-cleanup-directory");

        assert!(fs::rename(&cleanup, &swapped).is_err());
        drop(upgrade);
    }

    #[test]
    fn changed_backup_content_keeps_recovery_marker() {
        let fixture = project_fixture("changed-proof");
        let layout = fixture.layout();
        let mut upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
        let mut backup = upgrade.metadata_backup_file.try_clone().unwrap();
        let length = usize::try_from(backup.metadata().unwrap().len()).unwrap();
        backup.seek(SeekFrom::Start(0)).unwrap();
        backup.write_all(&vec![b'x'; length]).unwrap();
        backup.sync_all().unwrap();

        assert!(upgrade.commit().is_err());
        assert!(layout
            .root()
            .join(".project-upgrade-recovery.json")
            .is_file());
        assert!(layout
            .root()
            .join("recovery/metadata-schema-1.sqlite.backup")
            .is_file());
    }

    #[cfg(windows)]
    #[test]
    fn locked_restore_destination_keeps_recovery_proof() {
        use std::os::windows::fs::OpenOptionsExt;

        let fixture = project_fixture("locked-destination");
        let layout = fixture.layout();
        let mut upgrade = begin_project_upgrade(layout, CORRELATION_ID, MARKER).unwrap();
        fs::write(layout.metadata_path(), b"mutated metadata").unwrap();
        let locked = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(layout.metadata_path())
            .unwrap();

        assert!(upgrade.restore().is_err());
        assert!(layout
            .root()
            .join(".project-upgrade-recovery.json")
            .is_file());
        assert!(layout
            .root()
            .join("recovery/metadata-schema-1.sqlite.backup")
            .is_file());
        drop(locked);
        upgrade.restore().expect("restore after lock release");
    }

    #[test]
    fn project_namespace_directory_can_use_best_available_sync() {
        let fixture = project_fixture("directory-sync");
        sync_directory(fixture.layout().root()).expect("directory sync");
        sync_directory(&fixture.layout().root().join("recovery")).expect("recovery directory sync");
    }
}
