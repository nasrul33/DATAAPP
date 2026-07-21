use std::ffi::OsStr;
use std::path::{Component, Path};
use std::sync::Mutex;

use tauri::State;
use teratai_app_core::{ProjectError, ProjectErrorKind, ProjectService};
use teratai_contracts::generated::correlation_request::CorrelationRequest;
use teratai_contracts::generated::desktop_error::DesktopError;
use teratai_contracts::generated::project_create_request::ProjectCreateRequest;
use teratai_contracts::generated::project_descriptor::ProjectDescriptor;
use teratai_contracts::generated::project_open_request::ProjectOpenRequest;

type CommandResult<T> = Result<T, Box<DesktopError>>;

/// Owns the currently active project descriptor for the desktop process.
#[derive(Debug, Default)]
pub struct ProjectSession {
    current: Mutex<Option<ProjectDescriptor>>,
}

impl ProjectSession {
    fn create(&self, request: &ProjectCreateRequest) -> CommandResult<ProjectDescriptor> {
        validate_request_id(&request.request_id)?;
        let descriptor = ProjectService::create(request).map_err(|error| {
            map_project_error(&error, &request.request_id, ProjectOperation::Create)
        })?;
        self.activate(descriptor, &request.request_id)
    }

    fn open(&self, request: &ProjectOpenRequest) -> CommandResult<ProjectDescriptor> {
        validate_open_request(request)?;
        let descriptor =
            ProjectService::open(Path::new(&request.project_path)).map_err(|error| {
                map_project_error(&error, &request.request_id, ProjectOperation::Open)
            })?;
        self.activate(descriptor, &request.request_id)
    }

    fn validate(request: &ProjectOpenRequest) -> CommandResult<ProjectDescriptor> {
        validate_open_request(request)?;
        ProjectService::validate(Path::new(&request.project_path)).map_err(|error| {
            map_project_error(&error, &request.request_id, ProjectOperation::Validate)
        })
    }

    fn current(&self, request: &CorrelationRequest) -> CommandResult<Option<ProjectDescriptor>> {
        validate_request_id(&request.request_id)?;
        self.current
            .lock()
            .map(|current| current.clone())
            .map_err(|_| session_error(&request.request_id))
    }

    fn close(&self, request: &CorrelationRequest) -> CommandResult<()> {
        validate_request_id(&request.request_id)?;
        let mut current = self
            .current
            .lock()
            .map_err(|_| session_error(&request.request_id))?;
        *current = None;
        Ok(())
    }

    fn activate(
        &self,
        descriptor: ProjectDescriptor,
        correlation_id: &str,
    ) -> CommandResult<ProjectDescriptor> {
        let mut current = self
            .current
            .lock()
            .map_err(|_| session_error(correlation_id))?;
        *current = Some(descriptor.clone());
        Ok(descriptor)
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn project_create(
    request: ProjectCreateRequest,
    session: State<'_, ProjectSession>,
) -> CommandResult<ProjectDescriptor> {
    session.create(&request)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn project_open(
    request: ProjectOpenRequest,
    session: State<'_, ProjectSession>,
) -> CommandResult<ProjectDescriptor> {
    session.open(&request)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn project_validate(request: ProjectOpenRequest) -> CommandResult<ProjectDescriptor> {
    ProjectSession::validate(&request)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn project_current(
    request: CorrelationRequest,
    session: State<'_, ProjectSession>,
) -> CommandResult<Option<ProjectDescriptor>> {
    session.current(&request)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn project_close(
    request: CorrelationRequest,
    session: State<'_, ProjectSession>,
) -> CommandResult<()> {
    session.close(&request)
}

fn validate_open_request(request: &ProjectOpenRequest) -> CommandResult<()> {
    validate_request_id(&request.request_id)?;
    let path = Path::new(&request.project_path);
    let valid_extension = path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("teratai"));
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
        || !valid_extension
    {
        return Err(Box::new(DesktopError {
            code: "VALIDATION_ERROR".to_owned(),
            correlation_id: request.request_id.clone(),
            detail: "project path failed native boundary validation".to_owned(),
            field_errors: vec![
                "Pilih direktori proyek absolut dengan ekstensi .teratai.".to_owned()
            ],
            message: "Lokasi proyek tidak valid.".to_owned(),
            remediation: Some("Pilih ulang proyek melalui dialog Teratai.".to_owned()),
            retriable: false,
        }));
    }
    Ok(())
}

fn validate_request_id(request_id: &str) -> CommandResult<()> {
    if is_uuid_v7(request_id) {
        return Ok(());
    }
    Err(Box::new(DesktopError {
        code: "VALIDATION_ERROR".to_owned(),
        correlation_id: request_id.to_owned(),
        detail: "request correlation failed UUID v7 validation".to_owned(),
        field_errors: vec!["Identitas permintaan tidak valid.".to_owned()],
        message: "Permintaan proyek tidak valid.".to_owned(),
        remediation: Some("Ulangi tindakan dari aplikasi Teratai.".to_owned()),
        retriable: false,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectOperation {
    Create,
    Open,
    Validate,
}

fn map_project_error(
    error: &ProjectError,
    correlation_id: &str,
    operation: ProjectOperation,
) -> Box<DesktopError> {
    let specification = match (error.kind(), operation) {
        (ProjectErrorKind::InvalidRequest, _) => ErrorSpec::new(
            "VALIDATION_ERROR", "project request failed core validation", "Data proyek belum valid.",
            "Perbaiki isian lalu coba kembali.", false, Some("Periksa nama dan lokasi proyek."),
        ),
        (ProjectErrorKind::TargetExists, _) => ErrorSpec::new(
            "VALIDATION_ERROR", "project target already exists", "Proyek tidak dapat dibuat di lokasi tersebut.",
            "Pilih nama atau lokasi lain.", false, Some("Lokasi tujuan sudah digunakan."),
        ),
        (ProjectErrorKind::RecoveryRequired, _) => ErrorSpec::new(
            "PROJECT_CORRUPTED", "project creation recovery marker is present", "Proyek memerlukan pemulihan sebelum dapat dibuka.",
            "Jangan hapus berkas proyek. Gunakan alur pemulihan pada versi berikutnya atau hubungi administrator.", false, None,
        ),
        (ProjectErrorKind::PermissionDenied, _) => ErrorSpec::new(
            "PERMISSION_DENIED", "operating system denied project path access", "Teratai tidak memiliki izin untuk lokasi tersebut.",
            "Pilih lokasi yang dapat ditulis atau perbarui izin folder.", true, None,
        ),
        (ProjectErrorKind::InvalidPath, _) => ErrorSpec::new(
            "VALIDATION_ERROR", "project path failed filesystem policy", "Lokasi proyek tidak valid.",
            "Pilih ulang lokasi melalui dialog Teratai.", false, Some("Lokasi proyek tidak memenuhi kebijakan keamanan."),
        ),
        (ProjectErrorKind::IncompatibleProject, _) => ErrorSpec::new(
            "PROJECT_CORRUPTED", "project schema version is incompatible", "Versi proyek tidak kompatibel dengan Teratai ini.",
            "Buka proyek menggunakan versi Teratai yang sesuai.", false, None,
        ),
        (ProjectErrorKind::DataIntegrity, _) => ErrorSpec::new(
            "PROJECT_CORRUPTED", "project manifest and metadata integrity checks disagree", "Integritas proyek tidak dapat diverifikasi.",
            "Jangan mengubah proyek. Pulihkan dari salinan tepercaya.", false, None,
        ),
        (ProjectErrorKind::InvalidLayout, _) => ErrorSpec::new(
            "PROJECT_CORRUPTED", "required project layout is incomplete", "Struktur proyek tidak lengkap.",
            "Pilih proyek lain atau pulihkan dari salinan tepercaya.", false, None,
        ),
        (ProjectErrorKind::ControlFileTooLarge, _) => ErrorSpec::new(
            "PROJECT_CORRUPTED", "bounded project control file exceeds its limit", "Berkas kontrol proyek melewati batas aman.",
            "Pulihkan proyek dari salinan tepercaya.", false, None,
        ),
        (ProjectErrorKind::Filesystem, _) => ErrorSpec::new(
            "OPERATION_FAILED", "project filesystem operation failed", "Operasi penyimpanan proyek gagal.",
            "Pastikan media tersedia, lalu coba kembali.", true, None,
        ),
        (ProjectErrorKind::Database | ProjectErrorKind::Serialization | ProjectErrorKind::Timestamp, ProjectOperation::Create) => ErrorSpec::new(
            "OPERATION_FAILED", "project control data could not be initialized", "Penyimpanan proyek tidak dapat disiapkan.",
            "Pastikan media tersedia, lalu pilih lokasi lain.", true, None,
        ),
        (ProjectErrorKind::Database | ProjectErrorKind::Serialization | ProjectErrorKind::Timestamp, ProjectOperation::Open | ProjectOperation::Validate) => ErrorSpec::new(
            "PROJECT_CORRUPTED", "project control data validation failed", "Data kontrol proyek tidak dapat diverifikasi.",
            "Jangan ubah proyek. Pulihkan dari salinan tepercaya.", false, None,
        ),
    };
    Box::new(specification.into_error(correlation_id))
}

struct ErrorSpec {
    code: &'static str,
    detail: &'static str,
    message: &'static str,
    remediation: &'static str,
    retriable: bool,
    field_error: Option<&'static str>,
}

impl ErrorSpec {
    const fn new(
        code: &'static str,
        detail: &'static str,
        message: &'static str,
        remediation: &'static str,
        retriable: bool,
        field_error: Option<&'static str>,
    ) -> Self {
        Self {
            code,
            detail,
            message,
            remediation,
            retriable,
            field_error,
        }
    }

    fn into_error(self, correlation_id: &str) -> DesktopError {
        DesktopError {
            code: self.code.to_owned(),
            correlation_id: correlation_id.to_owned(),
            detail: self.detail.to_owned(),
            field_errors: self
                .field_error
                .map_or_else(Vec::new, |value| vec![value.to_owned()]),
            message: self.message.to_owned(),
            remediation: Some(self.remediation.to_owned()),
            retriable: self.retriable,
        }
    }
}

fn session_error(correlation_id: &str) -> Box<DesktopError> {
    Box::new(DesktopError {
        code: "OPERATION_FAILED".to_owned(),
        correlation_id: correlation_id.to_owned(),
        detail: "desktop project session lock is unavailable".to_owned(),
        field_errors: Vec::new(),
        message: "Sesi proyek tidak dapat diperbarui.".to_owned(),
        remediation: Some("Mulai ulang Teratai lalu buka kembali proyek.".to_owned()),
        retriable: true,
    })
}

fn is_uuid_v7(value: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000110";
    const REQUEST_ID: &str = "00000000-0000-7000-8000-000000000111";

    fn test_parent() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "teratai-desktop-session-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn creates_activates_and_closes_project_session() {
        let parent = test_parent();
        fs::create_dir(&parent).expect("test parent");
        let session = ProjectSession::default();
        let request = ProjectCreateRequest {
            name: "Audit Persediaan".to_owned(),
            project_id: PROJECT_ID.to_owned(),
            project_path: parent
                .join("Audit Persediaan.teratai")
                .to_string_lossy()
                .into_owned(),
            request_id: REQUEST_ID.to_owned(),
        };

        let created = session.create(&request).expect("create project");
        let correlation = CorrelationRequest {
            request_id: REQUEST_ID.to_owned(),
        };
        assert_eq!(
            session.current(&correlation).expect("current"),
            Some(created)
        );
        session.close(&correlation).expect("close");
        assert_eq!(session.current(&correlation).expect("current"), None);
        fs::remove_dir_all(parent).expect("test cleanup");
    }

    #[test]
    fn maps_core_failures_without_sensitive_details() {
        let validation_source =
            ProjectError::InvalidRequest("D:\\sensitive\\project.teratai".to_owned());
        let validation = map_project_error(&validation_source, REQUEST_ID, ProjectOperation::Open);
        let incompatible_source =
            ProjectError::IncompatibleProject("D:\\sensitive\\manifest.json".to_owned());
        let incompatible =
            map_project_error(&incompatible_source, REQUEST_ID, ProjectOperation::Open);

        assert_eq!(validation.code, "VALIDATION_ERROR");
        assert_eq!(incompatible.code, "PROJECT_CORRUPTED");
        assert!(!validation.detail.contains("sensitive"));
        assert!(!incompatible.detail.contains("sensitive"));
    }

    #[test]
    fn rejects_relative_open_path_at_native_boundary() {
        let error = validate_open_request(&ProjectOpenRequest {
            project_path: "relative.teratai".to_owned(),
            request_id: REQUEST_ID.to_owned(),
        })
        .expect_err("relative path must fail");

        assert_eq!(error.code, "VALIDATION_ERROR");
        assert_eq!(error.correlation_id, REQUEST_ID);
    }
}
