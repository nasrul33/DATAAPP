use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use teratai_app_core::{JobError, JobErrorKind, JobEventSink, JobLifecycleEvent, JobListCursor};
use teratai_contracts::generated::desktop_error::DesktopError;
use teratai_contracts::generated::job_descriptor::JobDescriptor;
use teratai_contracts::generated::job_get_request::JobGetRequest;
use teratai_contracts::generated::job_list_request::JobListRequest;
use teratai_contracts::generated::job_list_response::JobListResponse;
use teratai_contracts::generated::job_transition_request::JobTransitionRequest;

use crate::project_commands::{command_error, CommandResult, ProjectSession};

pub const JOB_LIFECYCLE_CHANNEL: &str = "job:lifecycle";

struct TauriJobEventSink {
    app: AppHandle,
}

impl JobEventSink for TauriJobEventSink {
    fn publish(&self, event: &JobLifecycleEvent) {
        let _ = self.app.emit(JOB_LIFECYCLE_CHANNEL, event);
    }
}

pub(crate) fn tauri_event_sink(app: AppHandle) -> Arc<dyn JobEventSink> {
    Arc::new(TauriJobEventSink { app })
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn job_get(
    request: JobGetRequest,
    session: State<'_, ProjectSession>,
) -> CommandResult<JobDescriptor> {
    get_job(&request, &session)
}

fn get_job(request: &JobGetRequest, session: &ProjectSession) -> CommandResult<JobDescriptor> {
    let store = session.job_store(&request.correlation_id)?;
    store
        .get(&request.job_id)
        .map_err(|error| map_job_error(&error, &request.correlation_id))
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn job_list(
    request: JobListRequest,
    session: State<'_, ProjectSession>,
) -> CommandResult<JobListResponse> {
    list_jobs(request, &session)
}

fn list_jobs(request: JobListRequest, session: &ProjectSession) -> CommandResult<JobListResponse> {
    if !(1..=100).contains(&request.limit) {
        return Err(validation_error(
            &request.correlation_id,
            "job list limit must be between one and one hundred",
            "Jumlah pekerjaan per halaman harus antara 1 dan 100.",
        ));
    }
    let limit = usize::try_from(request.limit).map_err(|_| {
        validation_error(
            &request.correlation_id,
            "job list limit cannot be represented safely",
            "Jumlah pekerjaan per halaman tidak valid.",
        )
    })?;
    let cursor = list_cursor(
        &request.correlation_id,
        request.cursor_updated_at,
        request.cursor_job_id,
    )?;
    let store = session.job_store(&request.correlation_id)?;
    let page = store
        .list(limit, cursor)
        .map_err(|error| map_job_error(&error, &request.correlation_id))?;
    let (next_cursor_updated_at, next_cursor_job_id) =
        page.next_cursor.map_or((None, None), |next| {
            (Some(next.updated_at), Some(next.job_id))
        });
    Ok(JobListResponse {
        items: page.items,
        next_cursor_job_id,
        next_cursor_updated_at,
    })
}

fn list_cursor(
    correlation_id: &str,
    cursor_updated_at: Option<String>,
    cursor_job_id: Option<String>,
) -> CommandResult<Option<JobListCursor>> {
    Ok(match (cursor_updated_at, cursor_job_id) {
        (None, None) => None,
        (Some(updated_at), Some(job_id)) => Some(JobListCursor { updated_at, job_id }),
        _ => {
            return Err(validation_error(
                correlation_id,
                "job list cursor fields must be supplied together",
                "Cursor halaman pekerjaan tidak lengkap.",
            ));
        }
    })
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn job_cancel(
    request: JobTransitionRequest,
    session: State<'_, ProjectSession>,
) -> CommandResult<JobDescriptor> {
    cancel_job(&request, &session)
}

fn cancel_job(
    request: &JobTransitionRequest,
    session: &ProjectSession,
) -> CommandResult<JobDescriptor> {
    let store = session.job_store(&request.correlation_id)?;
    store
        .request_cancellation(request)
        .map_err(|error| map_job_error(&error, &request.correlation_id))
}

pub(crate) fn map_job_error(error: &JobError, correlation_id: &str) -> Box<DesktopError> {
    match error.kind() {
        JobErrorKind::InvalidRequest => validation_error(
            correlation_id,
            "job request failed native validation",
            "Permintaan pekerjaan tidak valid.",
        ),
        JobErrorKind::JobNotFound => command_error(
            "VALIDATION_ERROR",
            correlation_id,
            "job identity does not exist in the active project",
            "Pekerjaan tidak ditemukan pada proyek aktif.",
            "Muat ulang daftar pekerjaan lalu coba kembali.",
            false,
        ),
        JobErrorKind::InvalidTransition => command_error(
            "OPERATION_FAILED",
            correlation_id,
            "requested job lifecycle transition is not permitted",
            "Status pekerjaan tidak mengizinkan tindakan tersebut.",
            "Muat ulang status pekerjaan sebelum mencoba kembali.",
            false,
        ),
        JobErrorKind::RevisionConflict => command_error(
            "OPERATION_FAILED",
            correlation_id,
            "job revision changed before the requested mutation",
            "Status pekerjaan telah berubah.",
            "Muat ulang pekerjaan lalu ulangi tindakan.",
            true,
        ),
        JobErrorKind::IncompatibleSchema => command_error(
            "PROJECT_UPGRADE_REQUIRED",
            correlation_id,
            "active project metadata schema does not support job runtime",
            "Proyek perlu ditingkatkan sebelum pekerjaan dapat digunakan.",
            "Gunakan alur peningkatan proyek sebelum membuka Job Center.",
            false,
        ),
        JobErrorKind::DataIntegrity => command_error(
            "PROJECT_CORRUPTED",
            correlation_id,
            "persistent job integrity verification failed",
            "Integritas data pekerjaan tidak dapat diverifikasi.",
            "Jangan ubah proyek. Pulihkan dari salinan tepercaya.",
            false,
        ),
        JobErrorKind::Database | JobErrorKind::Timestamp => command_error(
            "OPERATION_FAILED",
            correlation_id,
            "persistent job operation failed",
            "Operasi pekerjaan tidak dapat diselesaikan.",
            "Pastikan proyek tersedia lalu coba kembali.",
            true,
        ),
    }
}

fn validation_error(correlation_id: &str, detail: &str, message: &str) -> Box<DesktopError> {
    command_error(
        "VALIDATION_ERROR",
        correlation_id,
        detail,
        message,
        "Periksa permintaan lalu coba kembali.",
        false,
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use teratai_contracts::generated::job_enqueue_request::JobEnqueueRequest;
    use teratai_contracts::generated::project_create_request::ProjectCreateRequest;

    use super::*;

    const CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000401";
    const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000402";
    const JOB_ID: &str = "00000000-0000-7000-8000-000000000403";

    struct TestEventSink;

    impl JobEventSink for TestEventSink {
        fn publish(&self, _event: &JobLifecycleEvent) {}
    }

    fn test_parent() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "teratai-job-command-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn maps_job_errors_without_exposing_raw_details() {
        let source = JobError::DataIntegrity("D:\\sensitive\\metadata.sqlite".to_owned());
        let error = map_job_error(&source, CORRELATION_ID);

        assert_eq!(error.code, "PROJECT_CORRUPTED");
        assert_eq!(error.correlation_id, CORRELATION_ID);
        assert!(!error.detail.contains("sensitive"));
        assert!(!error.message.contains("sensitive"));
    }

    #[test]
    fn rejects_partial_list_cursor_before_store_access() {
        let error = list_cursor(
            CORRELATION_ID,
            None,
            Some("00000000-0000-7000-8000-000000000402".to_owned()),
        )
        .expect_err("partial cursor must fail");

        assert_eq!(error.code, "VALIDATION_ERROR");
        assert_eq!(error.correlation_id, CORRELATION_ID);
    }

    #[test]
    fn get_list_and_cancel_are_project_scoped_and_persistent() {
        let parent = test_parent();
        fs::create_dir(&parent).expect("test parent");
        let project_path = parent.join("Job IPC.teratai");
        let session = ProjectSession::default();
        session
            .create(
                &ProjectCreateRequest {
                    name: "Job IPC".to_owned(),
                    project_id: PROJECT_ID.to_owned(),
                    project_path: project_path.to_string_lossy().into_owned(),
                    request_id: CORRELATION_ID.to_owned(),
                },
                Arc::new(TestEventSink),
            )
            .expect("create active project");
        let store = session.job_store(CORRELATION_ID).expect("active job store");
        let queued = store
            .enqueue(&JobEnqueueRequest {
                correlation_id: CORRELATION_ID.to_owned(),
                job_id: JOB_ID.to_owned(),
                kind: "system.mock_long".to_owned(),
                progress_total: Some(10),
                progress_unit: Some("step".to_owned()),
            })
            .expect("enqueue job");

        let fetched = get_job(
            &JobGetRequest {
                correlation_id: CORRELATION_ID.to_owned(),
                job_id: JOB_ID.to_owned(),
            },
            &session,
        )
        .expect("get job");
        let listed = list_jobs(
            JobListRequest {
                correlation_id: CORRELATION_ID.to_owned(),
                limit: 25,
                cursor_job_id: None,
                cursor_updated_at: None,
            },
            &session,
        )
        .expect("list jobs");
        let cancelling = cancel_job(
            &JobTransitionRequest {
                correlation_id: CORRELATION_ID.to_owned(),
                expected_revision: queued.revision,
                job_id: JOB_ID.to_owned(),
            },
            &session,
        )
        .expect("cancel job");

        assert_eq!(fetched, queued);
        assert_eq!(listed.items, vec![queued]);
        assert!(listed.next_cursor_job_id.is_none());
        assert!(listed.next_cursor_updated_at.is_none());
        assert_eq!(cancelling.status, "CANCELLING");
        assert_eq!(
            store.get(JOB_ID).expect("persisted cancellation"),
            cancelling
        );

        drop(store);
        drop(session);
        fs::remove_dir_all(parent).expect("test cleanup");
    }
}
