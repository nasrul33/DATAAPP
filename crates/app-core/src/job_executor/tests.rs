use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::time::Duration;

use teratai_contracts::generated::project_create_request::ProjectCreateRequest;

use super::resource::{DurationClass, ResourceBudget, ResourceEstimate};
use super::{
    CheckpointDecision, ClockError, ExecutionContext, ExecutorClock, ExecutorCore, HandlerOutcome,
    JobExecutor, JobExecutorConfig, JobExecutorError, JobHandler, JobHandlerError, JobProgress,
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
    project_path: PathBuf,
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
            project_path,
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
        Ok("2099-07-21T00:00:00Z".to_owned())
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

struct GateHandler {
    entered: mpsc::SyncSender<i64>,
    release: Mutex<mpsc::Receiver<()>>,
    invocations: Arc<AtomicUsize>,
}

impl JobHandler for GateHandler {
    fn kind(&self) -> &'static str {
        "mock.success"
    }

    fn estimate(&self) -> ResourceEstimate {
        ResourceEstimate {
            memory_bytes: 16,
            disk_bytes: 32,
            duration: DurationClass::Short,
        }
    }

    fn run(&self, context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        for current in [1, 2] {
            self.entered.send(current).expect("signal checkpoint");
            self.release
                .lock()
                .expect("release receiver")
                .recv_timeout(Duration::from_secs(2))
                .expect("release checkpoint");
            if context
                .checkpoint(JobProgress {
                    current,
                    total: Some(2),
                    unit: Some("step".to_owned()),
                    phase: "mock.work".to_owned(),
                    message: format!("Langkah {current} dari 2"),
                })
                .expect("persist checkpoint")
                == CheckpointDecision::Cancelled
            {
                return Ok(HandlerOutcome::Cancelled);
            }
        }
        Ok(HandlerOutcome::Completed)
    }
}

#[test]
fn mock_job_persists_progress_and_succeeds() {
    let mut fixture = ExecutorFixture::new("mock-success");
    let queued = fixture.enqueue("mock.success");
    let invocations = Arc::new(AtomicUsize::new(0));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(2);
    let (release_sender, release_receiver) = mpsc::sync_channel(2);
    let handler = Arc::new(GateHandler {
        entered: entered_sender,
        release: Mutex::new(release_receiver),
        invocations: Arc::clone(&invocations),
    });
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![handler],
        ExecutorFixture::config(1),
        Arc::new(FixedClock),
    )
    .expect("start executor");

    executor.submit(&queued.job_id).expect("submit mock job");
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("handler enters first checkpoint"),
        1
    );
    let running = fixture.store().get(&queued.job_id).expect("running job");
    assert_eq!(running.status, "RUNNING");
    assert_eq!(running.revision, queued.revision + 1);

    release_sender.send(()).expect("release first checkpoint");
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("handler enters second checkpoint"),
        2
    );
    let first_progress = fixture.store().get(&queued.job_id).expect("first progress");
    assert_eq!(first_progress.progress_current, 1);
    assert_eq!(first_progress.revision, running.revision + 1);

    release_sender.send(()).expect("release second checkpoint");
    assert!(executor.core.queue.close_and_drain().is_empty());
    executor.workers[0]
        .completed
        .recv_timeout(Duration::from_secs(2))
        .expect("worker completes after closed queue");

    let succeeded = fixture.store().get(&queued.job_id).expect("succeeded job");
    assert_eq!(succeeded.status, "SUCCEEDED");
    assert_eq!(succeeded.progress_current, 2);
    assert_eq!(succeeded.progress_total, Some(2));
    assert_eq!(succeeded.progress_unit.as_deref(), Some("step"));
    assert_eq!(succeeded.progress_phase.as_deref(), Some("mock.work"));
    assert_eq!(succeeded.revision, first_progress.revision + 2);
    assert_eq!(invocations.load(Ordering::SeqCst), 1);

    let reopened = JobStore::open(&fixture.project_path).expect("reopen job store");
    assert_eq!(
        reopened.get(&queued.job_id).expect("reopened job"),
        succeeded
    );
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
fn concurrent_full_queue_serializes_transient_claim_and_rolls_back() {
    let mut fixture = ExecutorFixture::new("concurrent-bounded-admission");
    let queued = fixture.enqueue("passive.kind");
    let first_rejected = fixture.enqueue("passive.kind");
    let second_rejected = fixture.enqueue("passive.kind");
    let core = Arc::new(fixture.executor_core(vec![Arc::new(PassiveHandler)]));
    assert_eq!(core.admission_bound, 3);
    assert!(core.admitted.lock().expect("admission set").capacity() >= 3);
    core.claim_and_queue(&queued.job_id).expect("fill queue");

    let calls = Arc::new(AtomicUsize::new(0));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(2);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let release_receiver = Arc::new(Mutex::new(release_receiver));
    core.set_before_queue_push_hook(Arc::new({
        let calls = Arc::clone(&calls);
        let release_receiver = Arc::clone(&release_receiver);
        move || {
            let call = calls.fetch_add(1, Ordering::SeqCst);
            entered_sender.send(call).expect("signal transient claim");
            if call == 0 {
                release_receiver
                    .lock()
                    .expect("release receiver")
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release first transient claim");
            }
        }
    }))
    .expect("install admission hook");

    let first_core = Arc::clone(&core);
    let first_job_id = first_rejected.job_id.clone();
    let (result_sender, result_receiver) = mpsc::sync_channel(2);
    let first_result_sender = result_sender.clone();
    let first_thread = std::thread::spawn(move || {
        first_result_sender
            .send(first_core.claim_and_queue(&first_job_id))
            .expect("send first result");
    });
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("first transient claim enters"),
        0
    );

    let start = Arc::new(Barrier::new(2));
    let second_core = Arc::clone(&core);
    let second_job_id = second_rejected.job_id.clone();
    let second_start = Arc::clone(&start);
    let second_thread = std::thread::spawn(move || {
        second_start.wait();
        result_sender
            .send(second_core.claim_and_queue(&second_job_id))
            .expect("send second result");
    });
    start.wait();
    let second_before_release = entered_receiver.recv_timeout(Duration::from_millis(500));

    release_sender.send(()).expect("release first claim");
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("second transient claim enters after release"),
        1
    );
    let results = [
        result_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("first queue result"),
        result_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("second queue result"),
    ];
    first_thread.join().expect("first submission thread");
    second_thread.join().expect("second submission thread");

    assert!(matches!(
        second_before_release,
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(results
        .into_iter()
        .all(|result| matches!(result, Err(JobExecutorError::QueueFull))));
    let admitted = core.admitted.lock().expect("admission set");
    assert_eq!(admitted.len(), 1);
    assert!(admitted.contains(&queued.job_id));
    assert!(!admitted.contains(&first_rejected.job_id));
    assert!(!admitted.contains(&second_rejected.job_id));
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
