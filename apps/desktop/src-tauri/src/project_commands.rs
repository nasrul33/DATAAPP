use std::ffi::OsStr;
use std::path::{Component, Path};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, State};
use teratai_app_core::{
    JobError, JobErrorKind, JobEventSink, JobStore, ProjectError, ProjectErrorKind, ProjectService,
};
use teratai_contracts::generated::correlation_request::CorrelationRequest;
use teratai_contracts::generated::desktop_error::DesktopError;
use teratai_contracts::generated::project_create_request::ProjectCreateRequest;
use teratai_contracts::generated::project_descriptor::ProjectDescriptor;
use teratai_contracts::generated::project_open_request::ProjectOpenRequest;

pub(crate) type CommandResult<T> = Result<T, Box<DesktopError>>;

#[derive(Debug)]
struct ActiveProject {
    descriptor: ProjectDescriptor,
    job_store: Option<Arc<JobStore>>,
}

/// Owns the currently active project descriptor for the desktop process.
#[derive(Debug, Default)]
pub struct ProjectSession {
    lifecycle: Mutex<()>,
    current: Mutex<Option<ActiveProject>>,
}

impl ProjectSession {
    pub(crate) fn create(
        &self,
        request: &ProjectCreateRequest,
        event_sink: Arc<dyn JobEventSink>,
    ) -> CommandResult<ProjectDescriptor> {
        validate_request_id(&request.request_id)?;
        let _lifecycle = self.lock_lifecycle(&request.request_id)?;
        let descriptor = ProjectService::create(request).map_err(|error| {
            map_project_error(&error, &request.request_id, ProjectOperation::Create)
        })?;
        self.activate(descriptor, &request.request_id, event_sink)
    }

    pub(crate) fn open(
        &self,
        request: &ProjectOpenRequest,
        event_sink: Arc<dyn JobEventSink>,
    ) -> CommandResult<ProjectDescriptor> {
        validate_open_request(request)?;
        let _lifecycle = self.lock_lifecycle(&request.request_id)?;
        let descriptor =
            ProjectService::open(Path::new(&request.project_path)).map_err(|error| {
                map_project_error(&error, &request.request_id, ProjectOperation::Open)
            })?;
        self.activate(descriptor, &request.request_id, event_sink)
    }

    fn validate(request: &ProjectOpenRequest) -> CommandResult<ProjectDescriptor> {
        validate_open_request(request)?;
        ProjectService::validate(Path::new(&request.project_path)).map_err(|error| {
            map_project_error(&error, &request.request_id, ProjectOperation::Validate)
        })
    }

    fn current(&self, request: &CorrelationRequest) -> CommandResult<Option<ProjectDescriptor>> {
        validate_request_id(&request.request_id)?;
        self.descriptor_snapshot(&request.request_id)
    }

    fn descriptor_snapshot(
        &self,
        correlation_id: &str,
    ) -> CommandResult<Option<ProjectDescriptor>> {
        self.current
            .lock()
            .map(|current| current.as_ref().map(|active| active.descriptor.clone()))
            .map_err(|_| session_error(correlation_id))
    }

    fn close(&self, request: &CorrelationRequest) -> CommandResult<()> {
        validate_request_id(&request.request_id)?;
        let _lifecycle = self.lock_lifecycle(&request.request_id)?;
        let mut current = self
            .current
            .lock()
            .map_err(|_| session_error(&request.request_id))?;
        *current = None;
        Ok(())
    }

    pub(crate) fn upgrade(
        &self,
        request: &CorrelationRequest,
        event_sink: Arc<dyn JobEventSink>,
    ) -> CommandResult<ProjectDescriptor> {
        validate_request_id(&request.request_id)?;
        let _lifecycle = self.lock_lifecycle(&request.request_id)?;
        let descriptor = self
            .descriptor_snapshot(&request.request_id)?
            .ok_or_else(|| {
                command_error(
                    "VALIDATION_ERROR",
                    &request.request_id,
                    "no active project is available for upgrade",
                    "Tidak ada proyek aktif yang dapat ditingkatkan.",
                    "Buka proyek schema versi 1 lalu coba kembali.",
                    false,
                )
            })?;
        let upgraded =
            ProjectService::upgrade(Path::new(&descriptor.project_path), &request.request_id)
                .map_err(|error| {
                    map_project_error(&error, &request.request_id, ProjectOperation::Upgrade)
                })?;
        self.activate(upgraded, &request.request_id, event_sink)
    }

    fn lock_lifecycle(&self, correlation_id: &str) -> CommandResult<std::sync::MutexGuard<'_, ()>> {
        self.lifecycle
            .lock()
            .map_err(|_| session_error(correlation_id))
    }

    fn activate(
        &self,
        descriptor: ProjectDescriptor,
        correlation_id: &str,
        event_sink: Arc<dyn JobEventSink>,
    ) -> CommandResult<ProjectDescriptor> {
        let job_store = if descriptor.metadata_schema_version == 2 {
            Some(Arc::new(
                JobStore::open_with_event_sink(Path::new(&descriptor.project_path), event_sink)
                    .map_err(|error| map_project_activation_error(&error, correlation_id))?,
            ))
        } else {
            None
        };
        let mut current = self
            .current
            .lock()
            .map_err(|_| session_error(correlation_id))?;
        *current = Some(ActiveProject {
            descriptor: descriptor.clone(),
            job_store,
        });
        Ok(descriptor)
    }

    pub(crate) fn job_store(&self, correlation_id: &str) -> CommandResult<Arc<JobStore>> {
        validate_request_id(correlation_id)?;
        let current = self
            .current
            .lock()
            .map_err(|_| session_error(correlation_id))?;
        let active = current.as_ref().ok_or_else(|| {
            command_error(
                "VALIDATION_ERROR",
                correlation_id,
                "no active project is available for job access",
                "Buka proyek sebelum mengakses pekerjaan.",
                "Pilih atau buat proyek Teratai lalu coba kembali.",
                false,
            )
        })?;
        active.job_store.clone().ok_or_else(|| {
            command_error(
                "PROJECT_UPGRADE_REQUIRED",
                correlation_id,
                "active project metadata schema does not provide job runtime",
                "Proyek perlu ditingkatkan sebelum pekerjaan dapat digunakan.",
                "Gunakan alur peningkatan proyek sebelum membuka Job Center.",
                false,
            )
        })
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn project_create(
    request: ProjectCreateRequest,
    app: AppHandle,
    session: State<'_, ProjectSession>,
) -> CommandResult<ProjectDescriptor> {
    session.create(&request, crate::job_commands::tauri_event_sink(app))
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn project_open(
    request: ProjectOpenRequest,
    app: AppHandle,
    session: State<'_, ProjectSession>,
) -> CommandResult<ProjectDescriptor> {
    session.open(&request, crate::job_commands::tauri_event_sink(app))
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

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn project_upgrade(
    request: CorrelationRequest,
    app: AppHandle,
    session: State<'_, ProjectSession>,
) -> CommandResult<ProjectDescriptor> {
    session.upgrade(&request, crate::job_commands::tauri_event_sink(app))
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

pub(crate) fn validate_request_id(request_id: &str) -> CommandResult<()> {
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
    Upgrade,
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
            "PROJECT_CORRUPTED", "project recovery evidence requires explicit handling", "Proyek memerlukan pemulihan sebelum dapat dibuka.",
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
        (ProjectErrorKind::Database | ProjectErrorKind::Serialization | ProjectErrorKind::Timestamp, ProjectOperation::Upgrade) => ErrorSpec::new(
            "OPERATION_FAILED", "project control data upgrade failed after safe rollback", "Peningkatan proyek belum dapat diselesaikan.",
            "Pastikan media tersedia, lalu coba kembali.", true, None,
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

fn map_project_activation_error(error: &JobError, correlation_id: &str) -> Box<DesktopError> {
    match error.kind() {
        JobErrorKind::IncompatibleSchema | JobErrorKind::DataIntegrity => command_error(
            "PROJECT_CORRUPTED",
            correlation_id,
            "project job runtime activation could not verify durable schema or integrity",
            "Integritas metadata proyek tidak dapat diverifikasi.",
            "Jangan ubah proyek. Pulihkan dari salinan tepercaya.",
            false,
        ),
        JobErrorKind::Database | JobErrorKind::Timestamp => command_error(
            "OPERATION_FAILED",
            correlation_id,
            "project job runtime activation encountered a safe operational failure",
            "Runtime pekerjaan proyek belum dapat diaktifkan.",
            "Pastikan proyek tersedia, lalu coba kembali.",
            true,
        ),
        JobErrorKind::InvalidRequest
        | JobErrorKind::JobNotFound
        | JobErrorKind::InvalidTransition
        | JobErrorKind::RevisionConflict => command_error(
            "OPERATION_FAILED",
            correlation_id,
            "project job runtime activation returned an unsupported lifecycle failure",
            "Runtime pekerjaan proyek tidak dapat diaktifkan.",
            "Tutup lalu buka kembali proyek sebelum mencoba lagi.",
            false,
        ),
    }
}

pub(crate) fn command_error(
    code: &str,
    correlation_id: &str,
    detail: &str,
    message: &str,
    remediation: &str,
    retriable: bool,
) -> Box<DesktopError> {
    Box::new(DesktopError {
        code: code.to_owned(),
        correlation_id: correlation_id.to_owned(),
        detail: detail.to_owned(),
        field_errors: Vec::new(),
        message: message.to_owned(),
        remediation: Some(remediation.to_owned()),
        retriable,
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
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use teratai_app_core::test_utils::{
        create_schema_one_project_fixture, install_begin_project_upgrade_fault_for_test,
        BeginProjectUpgradeFault,
    };

    use super::*;

    const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000110";
    const REQUEST_ID: &str = "00000000-0000-7000-8000-000000000111";
    const CREATE_REQUEST_ID: &str = "00000000-0000-7000-8000-000000000112";

    struct TestEventSink;

    impl JobEventSink for TestEventSink {
        fn publish(&self, _event: &teratai_app_core::JobLifecycleEvent) {}
    }

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

    fn create_project_fixture(label: &str) -> (PathBuf, ProjectDescriptor) {
        let parent = test_parent().join(label);
        fs::create_dir_all(&parent).expect("test parent");
        let request = ProjectCreateRequest {
            name: "Audit Persediaan".to_owned(),
            project_id: PROJECT_ID.to_owned(),
            project_path: parent
                .join("Audit Persediaan.teratai")
                .to_string_lossy()
                .into_owned(),
            request_id: REQUEST_ID.to_owned(),
        };
        let descriptor = ProjectService::create(&request).expect("create project fixture");
        (parent, descriptor)
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

        let created = session
            .create(&request, Arc::new(TestEventSink))
            .expect("create project");
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

    #[test]
    fn schema_one_session_requires_explicit_upgrade_for_job_access() {
        let session = ProjectSession {
            lifecycle: Mutex::new(()),
            current: Mutex::new(Some(ActiveProject {
                descriptor: ProjectDescriptor {
                    created_at: "2026-07-24T01:00:00Z".to_owned(),
                    metadata_schema_version: 1,
                    name: "Project schema 1".to_owned(),
                    project_id: PROJECT_ID.to_owned(),
                    project_path: "D:\\Projects\\schema-one.teratai".to_owned(),
                    schema_version: "1.0.0".to_owned(),
                },
                job_store: None,
            })),
        };

        let error = session
            .job_store(REQUEST_ID)
            .expect_err("schema one job access must fail");

        assert_eq!(error.code, "PROJECT_UPGRADE_REQUIRED");
        assert_eq!(error.correlation_id, REQUEST_ID);
    }

    #[test]
    fn upgrade_requires_an_active_project() {
        let session = ProjectSession::default();
        let request = CorrelationRequest {
            request_id: REQUEST_ID.to_owned(),
        };

        let error = session
            .upgrade(&request, Arc::new(TestEventSink))
            .expect_err("missing active project must fail");

        assert_eq!(error.code, "VALIDATION_ERROR");
        assert!(!error.retriable);
    }

    #[test]
    fn upgrade_rejects_invalid_correlation_before_session_lookup() {
        let session = ProjectSession::default();
        let request = CorrelationRequest {
            request_id: "not-a-uuid".to_owned(),
        };

        let error = session
            .upgrade(&request, Arc::new(TestEventSink))
            .expect_err("invalid correlation must fail");

        assert_eq!(error.code, "VALIDATION_ERROR");
        assert!(!error.retriable);
    }

    #[test]
    fn schema_one_active_session_upgrades_and_activates_job_store() {
        let (parent, descriptor) = create_project_fixture("schema-one-active");
        let session = ProjectSession {
            lifecycle: Mutex::new(()),
            current: Mutex::new(Some(ActiveProject {
                descriptor: ProjectDescriptor {
                    metadata_schema_version: 1,
                    ..descriptor
                },
                job_store: None,
            })),
        };
        let request = CorrelationRequest {
            request_id: REQUEST_ID.to_owned(),
        };

        let upgraded = session
            .upgrade(&request, Arc::new(TestEventSink))
            .expect("upgrade project");

        assert_eq!(upgraded.metadata_schema_version, 2);
        assert_eq!(upgraded.project_id, PROJECT_ID);
        session
            .job_store(REQUEST_ID)
            .expect("upgrade must activate job store");
        drop(session);
        fs::remove_dir_all(parent).expect("test cleanup");
    }

    #[test]
    fn genuine_schema_one_upgrade_persists_audit_and_activates_job_store() {
        let fixture = create_schema_one_project_fixture(
            "desktop-genuine-schema-one",
            PROJECT_ID,
            CREATE_REQUEST_ID,
            "Audit Schema Satu",
            "2026-07-24T01:00:00Z",
        )
        .expect("create genuine schema-one project");
        let schema_one = ProjectService::open(fixture.path()).expect("open genuine schema one");
        assert_eq!(schema_one.metadata_schema_version, 1);
        let session = ProjectSession {
            lifecycle: Mutex::new(()),
            current: Mutex::new(Some(ActiveProject {
                descriptor: schema_one,
                job_store: None,
            })),
        };
        let request = CorrelationRequest {
            request_id: REQUEST_ID.to_owned(),
        };

        let upgraded = session
            .upgrade(&request, Arc::new(TestEventSink))
            .expect("upgrade genuine schema-one project");

        assert_eq!(upgraded.metadata_schema_version, 2);
        assert_eq!(
            ProjectService::open(fixture.path()).expect("reopen durable schema-two project"),
            upgraded
        );
        assert_eq!(
            fixture
                .audit_actions()
                .expect("read migration audit")
                .last(),
            Some(&"project.metadata_migrated".to_owned())
        );
        session
            .job_store(REQUEST_ID)
            .expect("genuine upgrade must activate a usable job store");
    }

    #[test]
    fn durable_schema_two_activation_integrity_failure_is_project_corrupted() {
        let fixture = create_schema_one_project_fixture(
            "desktop-activation-integrity",
            PROJECT_ID,
            CREATE_REQUEST_ID,
            "Audit Activation",
            "2026-07-24T01:00:00Z",
        )
        .expect("create genuine schema-one project");
        let upgraded = ProjectService::upgrade(fixture.path(), REQUEST_ID)
            .expect("durably upgrade fixture before activation");
        fixture
            .set_metadata_user_version(1)
            .expect("tamper fixture only after durable upgrade");
        let session = ProjectSession::default();

        let error = session
            .activate(upgraded, REQUEST_ID, Arc::new(TestEventSink))
            .expect_err("integrity-invalid job activation must fail");

        assert_eq!(error.code, "PROJECT_CORRUPTED");
        assert!(!error.retriable);
        assert!(!error.detail.contains("metadata.sqlite"));
        assert!(!error
            .detail
            .contains(fixture.path().to_string_lossy().as_ref()));
        assert_eq!(
            session
                .current(&CorrelationRequest {
                    request_id: REQUEST_ID.to_owned(),
                })
                .expect("activation failure must not publish a session"),
            None
        );
    }

    #[test]
    fn genuine_schema_one_cleanup_collision_maps_to_non_retriable_recovery() {
        let fixture = create_schema_one_project_fixture(
            "desktop-upgrade-cleanup-collision",
            PROJECT_ID,
            CREATE_REQUEST_ID,
            "Audit Recovery",
            "2026-07-24T01:00:00Z",
        )
        .expect("create genuine schema-one project");
        let schema_one = ProjectService::open(fixture.path()).expect("open genuine schema one");
        fs::create_dir(
            fixture
                .path()
                .join(format!(".project-upgrade-cleanup-{REQUEST_ID}")),
        )
        .expect("reserve cleanup collision");
        let session = ProjectSession {
            lifecycle: Mutex::new(()),
            current: Mutex::new(Some(ActiveProject {
                descriptor: schema_one.clone(),
                job_store: None,
            })),
        };
        let request = CorrelationRequest {
            request_id: REQUEST_ID.to_owned(),
        };

        let error = session
            .upgrade(&request, Arc::new(TestEventSink))
            .expect_err("unproven cleanup must require recovery");

        assert_eq!(error.code, "PROJECT_CORRUPTED");
        assert!(!error.retriable);
        assert!(!error.detail.contains("metadata.sqlite"));
        assert!(!error
            .detail
            .contains(fixture.path().to_string_lossy().as_ref()));
        assert_eq!(
            session.current(&request).expect("preserve active session"),
            Some(schema_one)
        );
        assert_eq!(
            ProjectService::open(fixture.path())
                .expect_err("recovery proof must remain visible")
                .kind(),
            ProjectErrorKind::RecoveryRequired
        );
    }

    #[test]
    fn genuine_schema_one_begin_failure_maps_to_non_retriable_recovery() {
        let fixture = create_schema_one_project_fixture(
            "desktop-upgrade-begin-recovery",
            PROJECT_ID,
            CREATE_REQUEST_ID,
            "Audit Begin Recovery",
            "2026-07-24T01:00:00Z",
        )
        .expect("create genuine schema-one project");
        let schema_one = ProjectService::open(fixture.path()).expect("open genuine schema one");
        let session = ProjectSession {
            lifecycle: Mutex::new(()),
            current: Mutex::new(Some(ActiveProject {
                descriptor: schema_one.clone(),
                job_store: None,
            })),
        };
        let request = CorrelationRequest {
            request_id: REQUEST_ID.to_owned(),
        };
        let _fault = install_begin_project_upgrade_fault_for_test(
            BeginProjectUpgradeFault::AfterMetadataBackup,
        );

        let error = session
            .upgrade(&request, Arc::new(TestEventSink))
            .expect_err("retained begin artifact must require recovery");

        assert_eq!(error.code, "PROJECT_CORRUPTED");
        assert!(!error.retriable);
        assert!(!error.detail.contains("metadata.sqlite"));
        assert!(!error
            .detail
            .contains(fixture.path().to_string_lossy().as_ref()));
        assert_eq!(
            session.current(&request).expect("preserve active session"),
            Some(schema_one)
        );
        assert_eq!(
            ProjectService::open(fixture.path())
                .expect_err("retained begin artifact must block project open")
                .kind(),
            ProjectErrorKind::RecoveryRequired
        );
    }

    #[test]
    fn schema_two_upgrade_is_idempotent_and_activates_missing_job_store() {
        let (parent, descriptor) = create_project_fixture("schema-two-active");
        let session = ProjectSession {
            lifecycle: Mutex::new(()),
            current: Mutex::new(Some(ActiveProject {
                descriptor: descriptor.clone(),
                job_store: None,
            })),
        };
        let request = CorrelationRequest {
            request_id: REQUEST_ID.to_owned(),
        };

        let upgraded = session
            .upgrade(&request, Arc::new(TestEventSink))
            .expect("upgrade project");

        assert_eq!(upgraded, descriptor);
        session
            .job_store(REQUEST_ID)
            .expect("upgrade must activate job store");
        drop(session);
        fs::remove_dir_all(parent).expect("test cleanup");
    }

    #[test]
    fn maps_upgrade_failures_without_sensitive_details() {
        let rollback_safe = map_project_error(
            &ProjectError::Timestamp("D:\\secret\\clock-state".to_owned()),
            REQUEST_ID,
            ProjectOperation::Upgrade,
        );
        assert_eq!(rollback_safe.code, "OPERATION_FAILED");
        assert!(rollback_safe.retriable);
        assert!(!rollback_safe.detail.contains("secret"));

        let recovery = map_project_error(
            &ProjectError::DataIntegrity("D:\\secret\\metadata.sqlite".to_owned()),
            REQUEST_ID,
            ProjectOperation::Upgrade,
        );
        assert_eq!(recovery.code, "PROJECT_CORRUPTED");
        assert!(!recovery.retriable);
        assert!(!recovery.detail.contains("secret"));
    }

    #[test]
    fn maps_project_activation_failures_to_safe_project_states() {
        let incompatible = map_project_activation_error(
            &JobError::IncompatibleSchema {
                expected: 2,
                actual: 1,
            },
            REQUEST_ID,
        );
        let integrity = map_project_activation_error(
            &JobError::DataIntegrity("D:\\secret\\metadata.sqlite".to_owned()),
            REQUEST_ID,
        );
        let operational = map_project_activation_error(
            &JobError::Timestamp("D:\\secret\\clock-state".to_owned()),
            REQUEST_ID,
        );

        assert_eq!(incompatible.code, "PROJECT_CORRUPTED");
        assert!(!incompatible.retriable);
        assert_eq!(integrity.code, "PROJECT_CORRUPTED");
        assert!(!integrity.retriable);
        assert!(!integrity.detail.contains("secret"));
        assert_eq!(operational.code, "OPERATION_FAILED");
        assert!(operational.retriable);
        assert!(!operational.detail.contains("secret"));
    }

    #[test]
    fn lifecycle_mutations_are_serialized() {
        let session = Arc::new(ProjectSession::default());
        let lifecycle = session.lifecycle.lock().expect("lifecycle guard");
        let competing_session = Arc::clone(&session);
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = competing_session.close(&CorrelationRequest {
                request_id: REQUEST_ID.to_owned(),
            });
            sender.send(result).expect("send close result");
        });

        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(lifecycle);
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("close result after lifecycle release")
            .expect("close succeeds");
        worker.join().expect("close worker");
    }
}
