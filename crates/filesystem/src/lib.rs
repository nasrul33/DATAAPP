#![doc = "Safe local filesystem capability boundary."]

use std::ffi::OsStr;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

/// Maximum supported project manifest size.
pub const MANIFEST_LIMIT_BYTES: u64 = 64 * 1024;
/// Directories created for every project before it becomes visible.
pub const REQUIRED_PROJECT_DIRECTORIES: [&str; 9] = [
    "data/source",
    "data/derived",
    "data/cache",
    "workflows",
    "findings",
    "exports",
    "attachments",
    "logs",
    "recovery",
];

const MANIFEST_FILE: &str = "manifest.json";
const METADATA_FILE: &str = "metadata.sqlite";
const RECOVERY_MARKER_FILE: &str = ".project-creation-recovery.json";

/// Typed failures returned by the project-scoped filesystem boundary.
#[derive(Debug)]
pub enum FilesystemError {
    /// The caller supplied an unsafe or unsupported path.
    InvalidPath(String),
    /// A project or interrupted creation already occupies the target.
    AlreadyExists(PathBuf),
    /// A recovery marker requires explicit validation before use.
    RecoveryRequired(PathBuf),
    /// A required project entry is absent or has the wrong type.
    InvalidLayout(String),
    /// A bounded file exceeded its contract limit.
    FileTooLarge { path: PathBuf, limit: u64 },
    /// The operating system rejected a filesystem operation.
    Io(io::Error),
}

impl Display for FilesystemError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(detail) => write!(formatter, "invalid project path: {detail}"),
            Self::AlreadyExists(path) => {
                write!(
                    formatter,
                    "project target already exists: {}",
                    path.display()
                )
            }
            Self::RecoveryRequired(path) => write!(
                formatter,
                "project recovery marker requires attention: {}",
                path.display()
            ),
            Self::InvalidLayout(detail) => write!(formatter, "invalid project layout: {detail}"),
            Self::FileTooLarge { path, limit } => write!(
                formatter,
                "bounded file exceeds {limit} bytes: {}",
                path.display()
            ),
            Self::Io(error) => write!(formatter, "filesystem operation failed: {error}"),
        }
    }
}

impl std::error::Error for FilesystemError {}

impl From<io::Error> for FilesystemError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Canonical paths for a validated project directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLayout {
    root: PathBuf,
}

impl ProjectLayout {
    /// Canonical project root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Project manifest path.
    #[must_use]
    pub fn manifest_path(&self) -> PathBuf {
        self.root.join(MANIFEST_FILE)
    }

    /// `SQLite` metadata path.
    #[must_use]
    pub fn metadata_path(&self) -> PathBuf {
        self.root.join(METADATA_FILE)
    }
}

/// In-progress project directory that is not visible at its final path yet.
#[derive(Debug)]
pub struct ProjectCreation {
    target_root: PathBuf,
    staging_root: PathBuf,
    correlation_id: String,
}

impl ProjectCreation {
    /// Temporary root used until every durable artifact has been validated.
    #[must_use]
    pub fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    /// `SQLite` path inside the staging directory.
    #[must_use]
    pub fn metadata_path(&self) -> PathBuf {
        self.staging_root.join(METADATA_FILE)
    }

    /// Atomically write the bounded project manifest within staging.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest cannot be durably written.
    pub fn write_manifest(&self, contents: &[u8]) -> Result<(), FilesystemError> {
        if u64::try_from(contents.len()).unwrap_or(u64::MAX) > MANIFEST_LIMIT_BYTES {
            return Err(FilesystemError::FileTooLarge {
                path: self.staging_root.join(MANIFEST_FILE),
                limit: MANIFEST_LIMIT_BYTES,
            });
        }
        atomic_write(
            &self.staging_root.join(MANIFEST_FILE),
            contents,
            &self.correlation_id,
        )
    }

    /// Publish the fully initialized directory with one same-volume rename.
    ///
    /// # Errors
    ///
    /// Returns an error when the target appeared concurrently, rename failed,
    /// or the recovery marker could not be cleared after publication.
    pub fn commit(self) -> Result<ProjectLayout, FilesystemError> {
        if self.target_root.exists() {
            return Err(FilesystemError::AlreadyExists(self.target_root));
        }
        fs::rename(&self.staging_root, &self.target_root)?;
        let marker = self.target_root.join(RECOVERY_MARKER_FILE);
        fs::remove_file(&marker)?;
        validate_project_layout(&self.target_root)
    }
}

/// Create a hidden, recovery-marked project staging directory.
///
/// # Errors
///
/// Returns an error for relative/traversal paths, unsupported extensions,
/// existing targets, interrupted prior attempts, or operating-system failures.
pub fn begin_project_creation(
    target: &Path,
    project_id: &str,
    correlation_id: &str,
    recovery_marker: &[u8],
) -> Result<ProjectCreation, FilesystemError> {
    validate_safe_token(project_id, "project_id")?;
    validate_safe_token(correlation_id, "correlation_id")?;
    if !target.is_absolute() || target.components().any(|part| part == Component::ParentDir) {
        return Err(FilesystemError::InvalidPath(
            "target must be absolute and cannot contain parent traversal".to_owned(),
        ));
    }
    if !target
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("teratai"))
    {
        return Err(FilesystemError::InvalidPath(
            "target directory must use the .teratai extension".to_owned(),
        ));
    }

    let file_name = target.file_name().ok_or_else(|| {
        FilesystemError::InvalidPath("target directory name is missing".to_owned())
    })?;
    if !target.file_stem().is_some_and(|stem| !stem.is_empty()) {
        return Err(FilesystemError::InvalidPath(
            "target directory name cannot be empty".to_owned(),
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| FilesystemError::InvalidPath("target parent is missing".to_owned()))?
        .canonicalize()?;
    let target_root = parent.join(file_name);
    if target_root.exists() {
        return Err(FilesystemError::AlreadyExists(target_root));
    }

    let staging_root = parent.join(format!(".teratai-{project_id}.creating"));
    if staging_root.exists() {
        return Err(FilesystemError::RecoveryRequired(
            staging_root.join(RECOVERY_MARKER_FILE),
        ));
    }
    fs::create_dir(&staging_root)?;
    if let Err(error) = atomic_write(
        &staging_root.join(RECOVERY_MARKER_FILE),
        recovery_marker,
        correlation_id,
    ) {
        let _ = fs::remove_dir(&staging_root);
        return Err(error);
    }
    for directory in REQUIRED_PROJECT_DIRECTORIES {
        fs::create_dir_all(staging_root.join(directory))?;
    }

    Ok(ProjectCreation {
        target_root,
        staging_root,
        correlation_id: correlation_id.to_owned(),
    })
}

/// Validate a project root without following project-controlled child paths.
///
/// # Errors
///
/// Returns an error when the root is non-canonical, recovery is pending, or a
/// required directory/file is missing.
pub fn validate_project_layout(root: &Path) -> Result<ProjectLayout, FilesystemError> {
    let canonical_root = root.canonicalize()?;
    if !canonical_root.is_dir() {
        return Err(FilesystemError::InvalidLayout(
            "project root is not a directory".to_owned(),
        ));
    }
    let marker = canonical_root.join(RECOVERY_MARKER_FILE);
    if marker.exists() {
        return Err(FilesystemError::RecoveryRequired(marker));
    }
    for directory in REQUIRED_PROJECT_DIRECTORIES {
        let entry = canonical_root.join(directory);
        let metadata = fs::symlink_metadata(&entry)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(FilesystemError::InvalidLayout(format!(
                "required directory is missing or linked: {directory}"
            )));
        }
    }
    for file in [MANIFEST_FILE, METADATA_FILE] {
        let entry = canonical_root.join(file);
        let metadata = fs::symlink_metadata(&entry)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(FilesystemError::InvalidLayout(format!(
                "required file is missing or linked: {file}"
            )));
        }
    }
    Ok(ProjectLayout {
        root: canonical_root,
    })
}

/// Read a small project control file with a hard byte limit.
///
/// # Errors
///
/// Returns an error when metadata cannot be read or exceeds the requested bound.
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, FilesystemError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(FilesystemError::InvalidLayout(
            "bounded control file must be a regular file".to_owned(),
        ));
    }
    if metadata.len() > limit {
        return Err(FilesystemError::FileTooLarge {
            path: path.to_owned(),
            limit,
        });
    }
    let mut contents = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut contents)?;
    if u64::try_from(contents.len()).unwrap_or(u64::MAX) > limit {
        return Err(FilesystemError::FileTooLarge {
            path: path.to_owned(),
            limit,
        });
    }
    Ok(contents)
}

fn atomic_write(path: &Path, contents: &[u8], token: &str) -> Result<(), FilesystemError> {
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| FilesystemError::InvalidPath("file name is not UTF-8".to_owned()))?;
    let temporary = path.with_file_name(format!(".{file_name}.{token}.tmp"));
    let write_result = (|| -> Result<(), io::Error> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result.map_err(FilesystemError::Io)
}

fn validate_safe_token(value: &str, field: &str) -> Result<(), FilesystemError> {
    let bytes = value.as_bytes();
    let is_uuid_v7 = bytes.len() == 36
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
        });
    if !is_uuid_v7 {
        return Err(FilesystemError::InvalidPath(format!(
            "{field} must be a lowercase UUID v7 token"
        )));
    }
    Ok(())
}

/// Identifies this crate as an initialized workspace component.
#[must_use]
pub const fn component_name() -> &'static str {
    "filesystem"
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000100";
    const REQUEST_ID: &str = "00000000-0000-7000-8000-000000000101";

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "teratai-filesystem-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn creates_and_commits_complete_project_layout() {
        let parent = test_root("commit");
        fs::create_dir(&parent).expect("test parent");
        let target = parent.join("Audit 2026.teratai");
        let creation =
            begin_project_creation(&target, PROJECT_ID, REQUEST_ID, br#"{"status":"creating"}"#)
                .expect("project staging");
        fs::write(creation.metadata_path(), b"sqlite").expect("metadata placeholder");
        creation
            .write_manifest(br#"{"schema_version":"1.0.0"}"#)
            .expect("manifest");

        let layout = creation.commit().expect("atomic project commit");

        assert_eq!(
            layout.root(),
            target.canonicalize().expect("canonical target")
        );
        assert!(layout.manifest_path().is_file());
        assert!(layout.metadata_path().is_file());
        fs::remove_dir_all(parent).expect("test cleanup");
    }

    #[test]
    fn rejects_relative_and_existing_targets() {
        assert!(matches!(
            begin_project_creation(Path::new("relative.teratai"), PROJECT_ID, REQUEST_ID, b"{}"),
            Err(FilesystemError::InvalidPath(_))
        ));

        let parent = test_root("existing");
        fs::create_dir(&parent).expect("test parent");
        let target = parent.join("existing.teratai");
        fs::create_dir(&target).expect("existing target");
        assert!(matches!(
            begin_project_creation(&target, PROJECT_ID, REQUEST_ID, b"{}"),
            Err(FilesystemError::AlreadyExists(_))
        ));
        fs::remove_dir_all(parent).expect("test cleanup");
    }

    #[test]
    fn exposes_component_name() {
        assert_eq!(component_name(), "filesystem");
    }
}
