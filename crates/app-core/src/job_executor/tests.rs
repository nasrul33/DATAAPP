use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use teratai_contracts::generated::project_create_request::ProjectCreateRequest;

use super::resource::{DurationClass, ResourceBudget, ResourceEstimate};
use super::{
    ClockError, ExecutionContext, ExecutorClock, ExecutorCore, HandlerOutcome, JobExecutorConfig,
    JobExecutorError, JobHandler, JobHandlerError,
};
use crate::job::JobTransitionRequest;
use crate::{JobEnqueueRequest, JobStore, ProjectService};

const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000301";
const CREATE_CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000302";
const UPGRADE_CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000303";
const JOB_CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000304";
static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct ExecutorFixture {
    store: Option<Arc<JobStore>>,
    parent: PathBuf,
    next_job: u64,
}

impl ExecutorFixture {
    fn new(label: &str) -> Self {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "teratai-executor-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&parent).expect("create fixture parent");
        let project_path = parent.join("executor.teratai");
        ProjectService::create(&ProjectCreateRequest {
            name: "Executor Test Project".to_owned(),
            project_id: PROJECT_ID.to_owned(),
            project_path: project_path.to_string_lossy().into_owned(),
            request_id: CREATE_CORRELATION_ID.to_owned(),
        })
        .expect("create fixture project");
        ProjectService::upgrade(&project_path, UPGRADE_CORRELATION_ID)
            .expect("upgrade fixture project");
        let store = Arc::new(JobStore::open(&project_path).expect("open fixture job store"));
        Self {
            store: Some(store),
            parent,
            next_job: 0x310,
        }
    }

    fn store(&self) -> &Arc<JobStore> {
        self.store.as_ref().expect("fixture store")
    }

    fn budget() -> ResourceBudget {
        ResourceBudget {
            memory_bytes: 1_024,
            disk_bytes: 2_048,
            max_duration: DurationClass::Medium,
        }
    }

    fn config(queue_capacity: usize) -> JobExecutorConfig {
        JobExecutorConfig {
            worker_count: 1,
            queue_capacity,
            resource_budget: Self::budget(),
            shutdown_timeout: Duration::from_secs(1),
        }
    }

    fn executor_core(&self, handlers: Vec<Arc<dyn JobHandler>>) -> ExecutorCore {
        ExecutorCore::new(
            Arc::clone(self.store()),
            handlers,
            Self::config(1),
            Arc::new(FixedClock),
        )
        .expect("valid executor core")
    }

    fn enqueue(&mut self, kind: &str) -> crate::JobDescriptor {
        let job_id = format!("00000000-0000-7000-8000-{0:012x}", self.next_job);
        self.next_job += 1;
        self.store()
            .enqueue(&JobEnqueueRequest {
                correlation_id: JOB_CORRELATION_ID.to_owned(),
                job_id,
                kind: kind.to_owned(),
                progress_total: None,
                progress_unit: None,
            })
            .expect("enqueue fixture job")
    }
}

impl Drop for ExecutorFixture {
    fn drop(&mut self) {
        drop(self.store.take());
        if self.parent.exists() {
            fs::remove_dir_all(&self.parent).expect("remove fixture project");
        }
    }
}

#[derive(Debug)]
struct FixedClock;

impl ExecutorClock for FixedClock {
    fn now(&self) -> Result<String, ClockError> {
        Ok("2026-07-21T00:00:00Z".to_owned())
    }
}

struct PassiveHandler;

impl JobHandler for PassiveHandler {
    fn kind(&self) -> &'static str {
        "passive.kind"
    }

    fn estimate(&self) -> ResourceEstimate {
        ResourceEstimate {
            memory_bytes: 1,
            disk_bytes: 1,
            duration: DurationClass::Short,
        }
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        Ok(HandlerOutcome::Completed)
    }
}

struct InvalidKindHandler;

impl JobHandler for InvalidKindHandler {
    fn kind(&self) -> &'static str {
        "Invalid.Kind"
    }

    fn estimate(&self) -> ResourceEstimate {
        PassiveHandler.estimate()
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        Ok(HandlerOutcome::Completed)
    }
}

#[test]
fn invalid_config_is_rejected_before_executor_core_construction() {
    let fixture = ExecutorFixture::new("invalid-config");
    let available = std::thread::available_parallelism()
        .expect("test parallelism")
        .get();
    let invalid_configs = [
        JobExecutorConfig {
            worker_count: 0,
            queue_capacity: 1,
            resource_budget: ExecutorFixture::budget(),
            shutdown_timeout: Duration::from_secs(1),
        },
        JobExecutorConfig {
            worker_count: 1,
            queue_capacity: 0,
            resource_budget: ExecutorFixture::budget(),
            shutdown_timeout: Duration::from_secs(1),
        },
        JobExecutorConfig {
            worker_count: 1,
            queue_capacity: 1,
            resource_budget: ExecutorFixture::budget(),
            shutdown_timeout: Duration::ZERO,
        },
        JobExecutorConfig {
            worker_count: available.saturating_add(1),
            queue_capacity: 1,
            resource_budget: ExecutorFixture::budget(),
            shutdown_timeout: Duration::from_secs(1),
        },
        JobExecutorConfig {
            worker_count: 1,
            queue_capacity: 1,
            resource_budget: ResourceBudget {
                memory_bytes: 0,
                ..ExecutorFixture::budget()
            },
            shutdown_timeout: Duration::from_secs(1),
        },
    ];
    for config in invalid_configs {
        assert!(matches!(
            ExecutorCore::new(
                Arc::clone(fixture.store()),
                Vec::new(),
                config,
                Arc::new(FixedClock),
            ),
            Err(JobExecutorError::InvalidConfiguration)
        ));
    }
}

#[test]
fn duplicate_or_invalid_handler_kind_rejects_construction() {
    let fixture = ExecutorFixture::new("handler-registry");
    assert!(matches!(
        ExecutorCore::new(
            Arc::clone(fixture.store()),
            vec![Arc::new(PassiveHandler), Arc::new(PassiveHandler)],
            ExecutorFixture::config(1),
            Arc::new(FixedClock),
        ),
        Err(JobExecutorError::InvalidConfiguration)
    ));
    assert!(matches!(
        ExecutorCore::new(
            Arc::clone(fixture.store()),
            vec![Arc::new(InvalidKindHandler)],
            ExecutorFixture::config(1),
            Arc::new(FixedClock),
        ),
        Err(JobExecutorError::InvalidConfiguration)
    ));
}

#[test]
fn unknown_kind_does_not_mutate_job() {
    let mut fixture = ExecutorFixture::new("admission");
    let queued = fixture.enqueue("known.kind");
    let core = fixture.executor_core(Vec::new());
    assert!(matches!(
        core.claim_and_queue(&queued.job_id),
        Err(JobExecutorError::HandlerNotFound)
    ));
    assert_eq!(fixture.store().get(&queued.job_id).expect("job"), queued);
}

#[test]
fn invalid_missing_or_nonqueued_job_is_classified_before_admission() {
    let mut fixture = ExecutorFixture::new("admission-validation");
    let queued = fixture.enqueue("passive.kind");
    let running = fixture
        .store()
        .start(&JobTransitionRequest {
            correlation_id: JOB_CORRELATION_ID.to_owned(),
            expected_revision: queued.revision,
            job_id: queued.job_id.clone(),
        })
        .expect("start fixture job");
    let core = fixture.executor_core(vec![Arc::new(PassiveHandler)]);

    assert!(matches!(
        core.claim_and_queue("not-a-uuid"),
        Err(JobExecutorError::InvalidJobId)
    ));
    assert!(matches!(
        core.claim_and_queue("00000000-0000-7000-8000-000000000399"),
        Err(JobExecutorError::JobNotFound)
    ));
    assert!(matches!(
        core.claim_and_queue(&running.job_id),
        Err(JobExecutorError::InvalidState)
    ));
}

#[test]
fn duplicate_and_full_queue_admission_leave_rejected_job_unchanged() {
    let mut fixture = ExecutorFixture::new("bounded-admission");
    let first = fixture.enqueue("passive.kind");
    let second = fixture.enqueue("passive.kind");
    let core = fixture.executor_core(vec![Arc::new(PassiveHandler)]);

    core.claim_and_queue(&first.job_id)
        .expect("first admission");
    assert!(matches!(
        core.claim_and_queue(&first.job_id),
        Err(JobExecutorError::AlreadySubmitted)
    ));
    assert!(matches!(
        core.claim_and_queue(&second.job_id),
        Err(JobExecutorError::QueueFull)
    ));
    assert!(!core
        .admitted
        .lock()
        .expect("admission set")
        .contains(&second.job_id));
    assert_eq!(
        fixture.store().get(&second.job_id).expect("second job"),
        second
    );
    assert_eq!(core.queue.pop().as_deref(), Some(first.job_id.as_str()));
    core.claim_and_queue(&second.job_id)
        .expect("failed queue push released admission claim");
}

#[test]
fn handler_error_accepts_only_bounded_safe_fields() {
    assert!(JobHandlerError::new("OPERATION_FAILED", "Operasi gagal.", false).is_ok());
    assert!(JobHandlerError::new("", "Operasi gagal.", false).is_err());
    assert!(JobHandlerError::new("invalid", "Operasi gagal.", false).is_err());
    assert!(JobHandlerError::new("OPERATION_FAILED", " pesan", false).is_err());
    assert!(JobHandlerError::new("OPERATION_FAILED", "pesan\n", false).is_err());
    assert!(JobHandlerError::new(&"A".repeat(121), "Operasi gagal.", false).is_err());
    assert!(JobHandlerError::new("OPERATION_FAILED", &"a".repeat(501), false).is_err());
}
