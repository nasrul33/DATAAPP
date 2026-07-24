use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::time::Duration;

use teratai_contracts::generated::project_create_request::ProjectCreateRequest;

use super::resource::{DurationClass, ResourceBudget, ResourceEstimate};
use super::{
    wait_for_join_exit, CheckpointDecision, ClockError, ExecutionContext, ExecutorClock,
    ExecutorCore, HandlerOutcome, JobExecutor, JobExecutorConfig, JobExecutorError, JobHandler,
    JobHandlerError, JobProgress, ReaperCommand, SystemExecutorClock, WorkerSlot,
};
use crate::job::{JobProgressUpdateRequest, JobTransitionRequest};
use crate::{JobEnqueueRequest, JobStore, ProjectService};

const PROJECT_ID: &str = "00000000-0000-7000-8000-000000000301";
const CREATE_CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000302";
const UPGRADE_CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000303";
const JOB_CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000304";
const RECOVERY_CORRELATION_ID: &str = "00000000-0000-7000-8000-000000000305";
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

struct CountingHandler {
    kind: &'static str,
    invocations: Arc<AtomicUsize>,
    estimates: Option<Arc<AtomicUsize>>,
    entered: Option<mpsc::SyncSender<()>>,
}

impl JobHandler for CountingHandler {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn estimate(&self) -> ResourceEstimate {
        if let Some(estimates) = &self.estimates {
            estimates.fetch_add(1, Ordering::SeqCst);
        }
        PassiveHandler.estimate()
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = &self.entered {
            entered.send(()).expect("signal handler invocation");
        }
        Ok(HandlerOutcome::Completed)
    }
}

struct BlockingCheckpointClock {
    calls: AtomicUsize,
    blocked: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

struct BoundedTestClock {
    calls: AtomicUsize,
    allowed_calls: usize,
    block_on: Option<usize>,
    blocked: Option<mpsc::SyncSender<()>>,
    release: Option<Mutex<mpsc::Receiver<()>>>,
}

impl ExecutorClock for BoundedTestClock {
    fn now(&self) -> Result<String, ClockError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call >= self.allowed_calls {
            return Err(ClockError);
        }
        if self.block_on == Some(call) {
            self.blocked
                .as_ref()
                .expect("blocked signal configured")
                .send(())
                .expect("signal blocked clock call");
            self.release
                .as_ref()
                .expect("clock release configured")
                .lock()
                .expect("clock release receiver")
                .recv_timeout(Duration::from_secs(2))
                .expect("release blocked clock call");
        }
        SystemExecutorClock.now()
    }
}

struct CompletionGateHandler {
    entered: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    invocations: Arc<AtomicUsize>,
}

struct ShutdownGateHandler {
    kind: &'static str,
    entered: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    invocations: Arc<AtomicUsize>,
}

impl JobHandler for ShutdownGateHandler {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn estimate(&self) -> ResourceEstimate {
        PassiveHandler.estimate()
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        self.entered.send(()).expect("signal shutdown gate");
        self.release
            .lock()
            .expect("shutdown release receiver")
            .recv_timeout(Duration::from_secs(10))
            .expect("release shutdown gate");
        Ok(HandlerOutcome::Completed)
    }
}

impl JobHandler for CompletionGateHandler {
    fn kind(&self) -> &'static str {
        "completion.gate"
    }

    fn estimate(&self) -> ResourceEstimate {
        PassiveHandler.estimate()
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        self.entered
            .send(())
            .expect("signal handler completion gate");
        self.release
            .lock()
            .expect("completion release receiver")
            .recv_timeout(Duration::from_secs(2))
            .expect("release handler completion");
        Ok(HandlerOutcome::Completed)
    }
}

impl ExecutorClock for BlockingCheckpointClock {
    fn now(&self) -> Result<String, ClockError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            self.blocked.send(()).expect("signal blocked checkpoint");
            self.release
                .lock()
                .expect("clock release receiver")
                .recv_timeout(Duration::from_secs(2))
                .expect("release blocked checkpoint");
        }
        SystemExecutorClock.now()
    }
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

struct OverBudgetHandler {
    invocations: Arc<AtomicUsize>,
}

impl JobHandler for OverBudgetHandler {
    fn kind(&self) -> &'static str {
        "resource.over-budget"
    }

    fn estimate(&self) -> ResourceEstimate {
        ResourceEstimate {
            memory_bytes: ExecutorFixture::budget().memory_bytes + 1,
            disk_bytes: 1,
            duration: DurationClass::Short,
        }
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(HandlerOutcome::Completed)
    }
}

struct FailingHandler;

impl JobHandler for FailingHandler {
    fn kind(&self) -> &'static str {
        "mock.failure"
    }

    fn estimate(&self) -> ResourceEstimate {
        PassiveHandler.estimate()
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        Err(
            JobHandlerError::new("MOCK_FAILED", "Pekerjaan mock gagal.", true)
                .expect("valid test failure"),
        )
    }
}

struct UnexpectedCancelledHandler;

impl JobHandler for UnexpectedCancelledHandler {
    fn kind(&self) -> &'static str {
        "mock.unexpected-cancelled"
    }

    fn estimate(&self) -> ResourceEstimate {
        PassiveHandler.estimate()
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        Ok(HandlerOutcome::Cancelled)
    }
}

struct PanicHandler;

impl JobHandler for PanicHandler {
    fn kind(&self) -> &'static str {
        "mock.panic"
    }

    fn estimate(&self) -> ResourceEstimate {
        PassiveHandler.estimate()
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        panic!("sentinel-secret-must-not-persist")
    }
}

struct FullBudgetFailingHandler {
    entered: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl JobHandler for FullBudgetFailingHandler {
    fn kind(&self) -> &'static str {
        "full-budget.failure"
    }

    fn estimate(&self) -> ResourceEstimate {
        ResourceEstimate {
            memory_bytes: ExecutorFixture::budget().memory_bytes,
            disk_bytes: ExecutorFixture::budget().disk_bytes,
            duration: DurationClass::Medium,
        }
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        self.entered.send(()).expect("signal failing handler");
        self.release
            .lock()
            .expect("failure release receiver")
            .recv_timeout(Duration::from_secs(2))
            .expect("release failing handler");
        Err(
            JobHandlerError::new("MOCK_FAILED", "Pekerjaan mock gagal.", true)
                .expect("valid test failure"),
        )
    }
}

struct FullBudgetSuccessHandler;

impl JobHandler for FullBudgetSuccessHandler {
    fn kind(&self) -> &'static str {
        "full-budget.success"
    }

    fn estimate(&self) -> ResourceEstimate {
        ResourceEstimate {
            memory_bytes: ExecutorFixture::budget().memory_bytes,
            disk_bytes: ExecutorFixture::budget().disk_bytes,
            duration: DurationClass::Medium,
        }
    }

    fn run(&self, _context: &mut ExecutionContext<'_>) -> Result<HandlerOutcome, JobHandlerError> {
        Ok(HandlerOutcome::Completed)
    }
}

fn job_history_counts(project_path: &std::path::Path, job_id: &str) -> (i64, i64) {
    let connection =
        rusqlite::Connection::open(project_path.join("metadata.sqlite")).expect("open metadata");
    let job_events = connection
        .query_row(
            "SELECT COUNT(*) FROM job_event WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .expect("count job events");
    let audit_events = connection
        .query_row(
            "SELECT COUNT(*) FROM audit_event
             WHERE target_type = 'job' AND target_id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .expect("count job audit events");
    (job_events, audit_events)
}

fn assert_job_audit_chain(fixture: &ExecutorFixture, descriptor: &crate::JobDescriptor) {
    let connection = rusqlite::Connection::open(fixture.project_path.join("metadata.sqlite"))
        .expect("open metadata for audit proof");
    let mut revision_statement = connection
        .prepare("SELECT revision FROM job_event WHERE job_id = ?1 ORDER BY sequence")
        .expect("prepare job revision query");
    let revisions = revision_statement
        .query_map([descriptor.job_id.as_str()], |row| row.get::<_, i64>(0))
        .expect("query job revisions")
        .collect::<Result<Vec<_>, _>>()
        .expect("decode job revisions");
    let expected_revisions = (1..=descriptor.revision).collect::<Vec<_>>();
    assert_eq!(revisions, expected_revisions);

    let mut audit_statement = connection
        .prepare(
            "SELECT before_hash, after_hash
             FROM audit_event
             WHERE target_type = 'job' AND target_id = ?1
             ORDER BY sequence",
        )
        .expect("prepare job audit query");
    let audit_hashes = audit_statement
        .query_map([descriptor.job_id.as_str()], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query job audit hashes")
        .collect::<Result<Vec<_>, _>>()
        .expect("decode job audit hashes");
    assert_eq!(audit_hashes.len(), revisions.len());
    assert_eq!(
        audit_hashes.first().and_then(|entry| entry.0.as_ref()),
        None
    );
    for hashes in audit_hashes.windows(2) {
        assert_eq!(
            hashes[0].1,
            hashes[1].0.as_deref().expect("linked before hash")
        );
    }
    let expected_terminal_hash =
        crate::job::snapshot_hash_for_test(descriptor).expect("hash terminal descriptor");
    assert_eq!(
        audit_hashes.last().map(|entry| entry.1.as_str()),
        Some(expected_terminal_hash.as_str())
    );
}

#[test]
fn over_budget_job_fails_preflight_without_invoking_handler() {
    let mut fixture = ExecutorFixture::new("over-budget-failure");
    let rejected = fixture.enqueue("resource.over-budget");
    let following = fixture.enqueue("following.success");
    let rejected_invocations = Arc::new(AtomicUsize::new(0));
    let following_invocations = Arc::new(AtomicUsize::new(0));
    let (following_sender, following_receiver) = mpsc::sync_channel(1);
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![
            Arc::new(OverBudgetHandler {
                invocations: Arc::clone(&rejected_invocations),
            }),
            Arc::new(CountingHandler {
                kind: "following.success",
                invocations: Arc::clone(&following_invocations),
                estimates: None,
                entered: Some(following_sender),
            }),
        ],
        ExecutorFixture::config(2),
        Arc::new(FixedClock),
    )
    .expect("start executor");

    executor
        .submit(&rejected.job_id)
        .expect("submit over-budget job");
    executor
        .submit(&following.job_id)
        .expect("submit following job");
    following_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("following handler proves preflight completed");
    assert!(executor.core.queue.close_and_drain().is_empty());
    executor.workers[0]
        .completed
        .recv_timeout(Duration::from_secs(2))
        .expect("worker completes after preflight failure");

    let preflight = fixture
        .store()
        .get(&rejected.job_id)
        .expect("preflight failure");
    assert_eq!(preflight.status, "FAILED");
    assert_eq!(preflight.error_code.as_deref(), Some("RESOURCE_LIMIT"));
    assert_eq!(
        preflight.error_message.as_deref(),
        Some("Pekerjaan melebihi batas resource yang dikonfigurasi.")
    );
    assert_eq!(preflight.error_retriable, Some(false));
    assert_eq!(rejected_invocations.load(Ordering::SeqCst), 0);
    assert_eq!(following_invocations.load(Ordering::SeqCst), 1);
    assert_job_audit_chain(&fixture, &preflight);
}

#[test]
fn poisoned_resource_ledger_is_an_internal_failure_not_a_resource_limit() {
    let mut fixture = ExecutorFixture::new("poisoned-resource-ledger");
    let job = fixture.enqueue("passive.kind");
    let core = fixture.executor_core(vec![Arc::new(PassiveHandler)]);
    core.resource_ledger.poison_for_test();

    super::execute_job(&core, &job.job_id).expect("normalize poisoned ledger");

    let failed = fixture.store().get(&job.job_id).expect("internal failure");
    assert_eq!(failed.status, "FAILED");
    assert_eq!(failed.error_code.as_deref(), Some("OPERATION_FAILED"));
    assert_ne!(failed.error_code.as_deref(), Some("RESOURCE_LIMIT"));
    assert_eq!(failed.error_retriable, Some(false));
    assert_job_audit_chain(&fixture, &failed);
}

#[test]
fn typed_handler_failure_preserves_validated_safe_fields() {
    let mut fixture = ExecutorFixture::new("typed-handler-failure");
    let failed_job = fixture.enqueue("mock.failure");
    let following = fixture.enqueue("following.success");
    let following_invocations = Arc::new(AtomicUsize::new(0));
    let (following_sender, following_receiver) = mpsc::sync_channel(1);
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![
            Arc::new(FailingHandler),
            Arc::new(CountingHandler {
                kind: "following.success",
                invocations: Arc::clone(&following_invocations),
                estimates: None,
                entered: Some(following_sender),
            }),
        ],
        ExecutorFixture::config(2),
        Arc::new(FixedClock),
    )
    .expect("start executor");

    executor
        .submit(&failed_job.job_id)
        .expect("submit failing job");
    executor
        .submit(&following.job_id)
        .expect("submit following job");
    following_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("following handler proves failure completed");
    assert!(executor.core.queue.close_and_drain().is_empty());
    executor.workers[0]
        .completed
        .recv_timeout(Duration::from_secs(2))
        .expect("worker completes after typed failure");

    let failed = fixture
        .store()
        .get(&failed_job.job_id)
        .expect("typed failure");
    assert_eq!(failed.status, "FAILED");
    assert_eq!(failed.error_code.as_deref(), Some("MOCK_FAILED"));
    assert_eq!(
        failed.error_message.as_deref(),
        Some("Pekerjaan mock gagal.")
    );
    assert_eq!(failed.error_retriable, Some(true));
    assert_eq!(following_invocations.load(Ordering::SeqCst), 1);
    assert_job_audit_chain(&fixture, &failed);
}

#[test]
fn unexpected_cancelled_outcome_becomes_safe_terminal_failure() {
    let mut fixture = ExecutorFixture::new("unexpected-cancelled-outcome");
    let invalid_job = fixture.enqueue("mock.unexpected-cancelled");
    let following_job = fixture.enqueue("following.success");
    let following_invocations = Arc::new(AtomicUsize::new(0));
    let (following_sender, following_receiver) = mpsc::sync_channel(1);
    let mut executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![
            Arc::new(UnexpectedCancelledHandler),
            Arc::new(CountingHandler {
                kind: "following.success",
                invocations: Arc::clone(&following_invocations),
                estimates: None,
                entered: Some(following_sender),
            }),
        ],
        ExecutorFixture::config(2),
        Arc::new(FixedClock),
    )
    .expect("start executor");

    executor
        .submit(&invalid_job.job_id)
        .expect("submit invalid cancellation outcome");
    executor
        .submit(&following_job.job_id)
        .expect("submit following job");
    following_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("worker continues after invalid handler outcome");
    executor.shutdown().expect("join worker");

    let invalid = fixture
        .store()
        .get(&invalid_job.job_id)
        .expect("invalid outcome terminal snapshot");
    let following = fixture
        .store()
        .get(&following_job.job_id)
        .expect("following terminal snapshot");
    assert_eq!(invalid.status, "FAILED");
    assert_eq!(invalid.error_code.as_deref(), Some("OPERATION_FAILED"));
    assert_eq!(
        invalid.error_message.as_deref(),
        Some("Handler job mengembalikan pembatalan tanpa permintaan aktif.")
    );
    assert_eq!(invalid.error_retriable, Some(false));
    assert_eq!(following.status, "SUCCEEDED");
    assert_eq!(following_invocations.load(Ordering::SeqCst), 1);
    assert_job_audit_chain(&fixture, &invalid);
    assert_job_audit_chain(&fixture, &following);
}

#[test]
fn failed_failure_transaction_releases_admission_and_full_resource_budget() {
    let mut fixture = ExecutorFixture::new("failure-transaction-cleanup");
    let failing_job = fixture.enqueue("full-budget.failure");
    let following_job = fixture.enqueue("full-budget.success");
    let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let (completed_sender, completed_receiver) = mpsc::sync_channel(2);
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![
            Arc::new(FullBudgetFailingHandler {
                entered: entered_sender,
                release: Mutex::new(release_receiver),
            }),
            Arc::new(FullBudgetSuccessHandler),
        ],
        ExecutorFixture::config(2),
        Arc::new(FixedClock),
    )
    .expect("start executor");
    executor
        .core
        .set_after_job_hook(Arc::new(move |job_id| {
            completed_sender
                .send(job_id.to_owned())
                .expect("signal completed worker iteration");
        }))
        .expect("install after-job hook");

    executor
        .submit(&failing_job.job_id)
        .expect("submit full-budget failing job");
    entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("failing handler holds full reservation");
    let running = fixture
        .store()
        .get(&failing_job.job_id)
        .expect("running snapshot before injected failure");
    let history_before = job_history_counts(&fixture.project_path, &failing_job.job_id);
    let audit_fault = crate::job::install_audit_insert_fault_for_test("full-budget.failure");
    release_sender.send(()).expect("release failing handler");
    assert_eq!(
        completed_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("failed persistence iteration completes"),
        failing_job.job_id
    );

    assert_eq!(
        fixture
            .store()
            .get(&failing_job.job_id)
            .expect("failure transaction rolled back"),
        running
    );
    assert_eq!(
        job_history_counts(&fixture.project_path, &failing_job.job_id),
        history_before
    );
    assert!(!executor
        .core
        .admitted
        .lock()
        .expect("admission set")
        .contains(&failing_job.job_id));

    drop(audit_fault);
    executor
        .submit(&following_job.job_id)
        .expect("submit following full-budget job");
    assert_eq!(
        completed_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("following full-budget job completes"),
        following_job.job_id
    );
    assert_eq!(
        fixture
            .store()
            .get(&following_job.job_id)
            .expect("following terminal snapshot")
            .status,
        "SUCCEEDED"
    );
}

#[test]
fn panic_is_sanitized_and_same_worker_runs_following_job() {
    let mut fixture = ExecutorFixture::new("panic-containment");
    let panicking_job = fixture.enqueue("mock.panic");
    let following_job = fixture.enqueue("following.success");
    let following_invocations = Arc::new(AtomicUsize::new(0));
    let (following_sender, following_receiver) = mpsc::sync_channel(1);
    let (panic_sender, panic_receiver) = mpsc::sync_channel(1);
    let _panic_observer =
        super::set_handler_panic_observer_for_test(panic_sender).expect("install panic observer");
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![
            Arc::new(PanicHandler),
            Arc::new(CountingHandler {
                kind: "following.success",
                invocations: Arc::clone(&following_invocations),
                estimates: None,
                entered: Some(following_sender),
            }),
        ],
        ExecutorFixture::config(2),
        Arc::new(FixedClock),
    )
    .expect("start executor");

    executor
        .submit(&panicking_job.job_id)
        .expect("submit panicking job");
    executor
        .submit(&following_job.job_id)
        .expect("submit following job");
    following_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("same worker survives panic");
    assert!(executor.core.queue.close_and_drain().is_empty());
    executor.workers[0]
        .completed
        .recv_timeout(Duration::from_secs(2))
        .expect("worker completes after contained panic");

    let panicked = fixture
        .store()
        .get(&panicking_job.job_id)
        .expect("panicked job");
    let following = fixture
        .store()
        .get(&following_job.job_id)
        .expect("following job");
    let emitted = panic_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("executor hook emits sanitized panic");
    assert_eq!(emitted, "Handler job gagal secara internal.");
    assert!(!emitted.contains("sentinel"));
    assert_eq!(panicked.status, "FAILED");
    assert_eq!(panicked.error_code.as_deref(), Some("OPERATION_FAILED"));
    assert_eq!(
        panicked.error_message.as_deref(),
        Some("Pekerjaan gagal karena kesalahan operasi internal.")
    );
    assert!(!panicked
        .error_message
        .as_deref()
        .unwrap_or_default()
        .contains("sentinel"));
    assert_eq!(panicked.error_retriable, Some(false));
    assert_eq!(following.status, "SUCCEEDED");
    assert_eq!(following_invocations.load(Ordering::SeqCst), 1);
    assert_job_audit_chain(&fixture, &panicked);
    assert_job_audit_chain(&fixture, &following);
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
    assert_job_audit_chain(&fixture, &succeeded);

    let reopened = JobStore::open(&fixture.project_path).expect("reopen job store");
    assert_eq!(
        reopened.get(&queued.job_id).expect("reopened job"),
        succeeded
    );
}

#[test]
fn queued_cancellation_skips_handler_and_completes_cancelled() {
    let mut fixture = ExecutorFixture::new("queued-cancellation");
    let blocker = fixture.enqueue("mock.success");
    let cancelled = fixture.enqueue("cancel.queued");
    let sentinel = fixture.enqueue("sentinel.kind");
    let blocker_invocations = Arc::new(AtomicUsize::new(0));
    let cancelled_invocations = Arc::new(AtomicUsize::new(0));
    let cancelled_estimates = Arc::new(AtomicUsize::new(0));
    let sentinel_invocations = Arc::new(AtomicUsize::new(0));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(2);
    let (release_sender, release_receiver) = mpsc::sync_channel(2);
    let (sentinel_sender, sentinel_receiver) = mpsc::sync_channel(1);
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![
            Arc::new(GateHandler {
                entered: entered_sender,
                release: Mutex::new(release_receiver),
                invocations: Arc::clone(&blocker_invocations),
            }),
            Arc::new(CountingHandler {
                kind: "cancel.queued",
                invocations: Arc::clone(&cancelled_invocations),
                estimates: Some(Arc::clone(&cancelled_estimates)),
                entered: None,
            }),
            Arc::new(CountingHandler {
                kind: "sentinel.kind",
                invocations: Arc::clone(&sentinel_invocations),
                estimates: None,
                entered: Some(sentinel_sender),
            }),
        ],
        ExecutorFixture::config(2),
        Arc::new(FixedClock),
    )
    .expect("start executor");

    executor.submit(&blocker.job_id).expect("submit blocker");
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("blocker enters first checkpoint"),
        1
    );
    executor
        .submit(&cancelled.job_id)
        .expect("submit queued cancellation target");
    executor.submit(&sentinel.job_id).expect("submit sentinel");
    let cancelling = fixture
        .store()
        .request_cancellation(&JobTransitionRequest {
            correlation_id: cancelled.correlation_id.clone(),
            expected_revision: cancelled.revision,
            job_id: cancelled.job_id.clone(),
        })
        .expect("request queued cancellation");

    release_sender
        .send(())
        .expect("release blocker first checkpoint");
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("blocker enters second checkpoint"),
        2
    );
    release_sender
        .send(())
        .expect("release blocker second checkpoint");
    sentinel_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("worker advances past cancelled job");
    assert!(executor.core.queue.close_and_drain().is_empty());
    executor.workers[0]
        .completed
        .recv_timeout(Duration::from_secs(2))
        .expect("worker completes after closed queue");

    let terminal = fixture
        .store()
        .get(&cancelled.job_id)
        .expect("cancelled queued job");
    assert_eq!(terminal.status, "CANCELLED");
    assert_eq!(terminal.revision, cancelling.revision + 1);
    assert_eq!(cancelled_invocations.load(Ordering::SeqCst), 0);
    assert_eq!(cancelled_estimates.load(Ordering::SeqCst), 0);
    assert_eq!(blocker_invocations.load(Ordering::SeqCst), 1);
    assert_eq!(sentinel_invocations.load(Ordering::SeqCst), 1);
    assert_job_audit_chain(&fixture, &terminal);
}

#[test]
fn running_cancellation_stops_before_second_work_unit_without_duplicate_event() {
    let mut fixture = ExecutorFixture::new("running-cancellation");
    let job = fixture.enqueue("mock.success");
    let invocations = Arc::new(AtomicUsize::new(0));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(2);
    let (release_sender, release_receiver) = mpsc::sync_channel(2);
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![Arc::new(GateHandler {
            entered: entered_sender,
            release: Mutex::new(release_receiver),
            invocations: Arc::clone(&invocations),
        })],
        ExecutorFixture::config(1),
        Arc::new(SystemExecutorClock),
    )
    .expect("start executor");

    executor
        .submit(&job.job_id)
        .expect("submit cancellable job");
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("handler enters first checkpoint"),
        1
    );
    let running = fixture.store().get(&job.job_id).expect("running snapshot");
    let cancelling = fixture
        .store()
        .request_cancellation(&JobTransitionRequest {
            job_id: job.job_id.clone(),
            correlation_id: job.correlation_id.clone(),
            expected_revision: running.revision,
        })
        .expect("request cancellation");
    let repeated = fixture
        .store()
        .request_cancellation(&JobTransitionRequest {
            job_id: job.job_id.clone(),
            correlation_id: job.correlation_id.clone(),
            expected_revision: cancelling.revision,
        })
        .expect("repeat cancellation");
    assert_eq!(repeated, cancelling);

    release_sender
        .send(())
        .expect("release cancelled checkpoint");
    assert!(executor.core.queue.close_and_drain().is_empty());
    executor.workers[0]
        .completed
        .recv_timeout(Duration::from_secs(2))
        .expect("worker completes after cancellation");

    let terminal = fixture.store().get(&job.job_id).expect("cancelled job");
    assert_eq!(terminal.status, "CANCELLED");
    assert_eq!(terminal.progress_current, 0);
    assert_eq!(terminal.revision, cancelling.revision + 1);
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert!(matches!(
        entered_receiver.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Disconnected | mpsc::RecvTimeoutError::Timeout)
    ));
    assert_job_audit_chain(&fixture, &terminal);
}

#[test]
fn cancellation_winning_progress_cas_is_reconciled_once_without_progress_write() {
    let mut fixture = ExecutorFixture::new("cancellation-progress-cas");
    let job = fixture.enqueue("mock.success");
    let invocations = Arc::new(AtomicUsize::new(0));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(2);
    let (release_sender, release_receiver) = mpsc::sync_channel(2);
    let (clock_blocked_sender, clock_blocked_receiver) = mpsc::sync_channel(1);
    let (clock_release_sender, clock_release_receiver) = mpsc::sync_channel(1);
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![Arc::new(GateHandler {
            entered: entered_sender,
            release: Mutex::new(release_receiver),
            invocations: Arc::clone(&invocations),
        })],
        ExecutorFixture::config(1),
        Arc::new(BlockingCheckpointClock {
            calls: AtomicUsize::new(0),
            blocked: clock_blocked_sender,
            release: Mutex::new(clock_release_receiver),
        }),
    )
    .expect("start executor");

    executor
        .submit(&job.job_id)
        .expect("submit cancellable job");
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("handler enters first checkpoint"),
        1
    );
    release_sender.send(()).expect("start checkpoint write");
    clock_blocked_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("checkpoint blocks after reading running revision");
    let running = fixture.store().get(&job.job_id).expect("running snapshot");
    let cancelling = fixture
        .store()
        .request_cancellation(&JobTransitionRequest {
            job_id: job.job_id.clone(),
            correlation_id: job.correlation_id.clone(),
            expected_revision: running.revision,
        })
        .expect("cancellation wins progress CAS");
    clock_release_sender
        .send(())
        .expect("release stale progress write");
    assert!(executor.core.queue.close_and_drain().is_empty());
    executor.workers[0]
        .completed
        .recv_timeout(Duration::from_secs(2))
        .expect("worker reconciles cancellation");

    let terminal = fixture.store().get(&job.job_id).expect("cancelled job");
    assert_eq!(terminal.status, "CANCELLED");
    assert_eq!(terminal.progress_current, 0);
    assert_eq!(terminal.revision, cancelling.revision + 1);
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert!(matches!(
        entered_receiver.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Disconnected | mpsc::RecvTimeoutError::Timeout)
    ));
}

#[test]
fn completed_handler_observes_existing_cancellation_without_extra_clock_call() {
    let mut fixture = ExecutorFixture::new("completed-handler-cancelling");
    let job = fixture.enqueue("completion.gate");
    let invocations = Arc::new(AtomicUsize::new(0));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let clock = Arc::new(BoundedTestClock {
        calls: AtomicUsize::new(0),
        allowed_calls: 2,
        block_on: None,
        blocked: None,
        release: None,
    });
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![Arc::new(CompletionGateHandler {
            entered: entered_sender,
            release: Mutex::new(release_receiver),
            invocations: Arc::clone(&invocations),
        })],
        ExecutorFixture::config(1),
        clock.clone(),
    )
    .expect("start executor");

    executor
        .submit(&job.job_id)
        .expect("submit completion gate");
    entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("handler waits before completed outcome");
    let running = fixture.store().get(&job.job_id).expect("running snapshot");
    let cancelling = fixture
        .store()
        .request_cancellation(&JobTransitionRequest {
            job_id: job.job_id.clone(),
            correlation_id: job.correlation_id.clone(),
            expected_revision: running.revision,
        })
        .expect("request cancellation before handler completion");
    release_sender
        .send(())
        .expect("release completed handler outcome");
    assert!(executor.core.queue.close_and_drain().is_empty());
    executor.workers[0]
        .completed
        .recv_timeout(Duration::from_secs(2))
        .expect("worker completes cancellation without extra clock call");

    let terminal = fixture.store().get(&job.job_id).expect("cancelled job");
    assert_eq!(terminal.status, "CANCELLED");
    assert_eq!(terminal.revision, cancelling.revision + 1);
    assert_eq!(clock.calls.load(Ordering::SeqCst), 2);
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_winning_success_cas_uses_one_completion_timestamp() {
    let mut fixture = ExecutorFixture::new("cancellation-success-cas");
    let job = fixture.enqueue("passive.kind");
    let (clock_blocked_sender, clock_blocked_receiver) = mpsc::sync_channel(1);
    let (clock_release_sender, clock_release_receiver) = mpsc::sync_channel(1);
    let clock = Arc::new(BoundedTestClock {
        calls: AtomicUsize::new(0),
        allowed_calls: 3,
        block_on: Some(1),
        blocked: Some(clock_blocked_sender),
        release: Some(Mutex::new(clock_release_receiver)),
    });
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![Arc::new(PassiveHandler)],
        ExecutorFixture::config(1),
        clock.clone(),
    )
    .expect("start executor");

    executor.submit(&job.job_id).expect("submit passive job");
    clock_blocked_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("success path blocks after snapshot read");
    let running = fixture.store().get(&job.job_id).expect("running snapshot");
    let cancelling = fixture
        .store()
        .request_cancellation(&JobTransitionRequest {
            job_id: job.job_id.clone(),
            correlation_id: job.correlation_id.clone(),
            expected_revision: running.revision,
        })
        .expect("cancellation wins success CAS");
    clock_release_sender
        .send(())
        .expect("release stale success write");
    assert!(executor.core.queue.close_and_drain().is_empty());
    executor.workers[0]
        .completed
        .recv_timeout(Duration::from_secs(2))
        .expect("worker reconciles success conflict");

    let terminal = fixture.store().get(&job.job_id).expect("cancelled job");
    assert_eq!(terminal.status, "CANCELLED");
    assert_eq!(terminal.revision, cancelling.revision + 1);
    assert_eq!(clock.calls.load(Ordering::SeqCst), 3);
}

#[test]
fn running_progress_cas_conflict_retries_only_the_same_progress_once() {
    let mut fixture = ExecutorFixture::new("running-progress-cas");
    let job = fixture.enqueue("mock.success");
    let invocations = Arc::new(AtomicUsize::new(0));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(2);
    let (release_sender, release_receiver) = mpsc::sync_channel(2);
    let (clock_blocked_sender, clock_blocked_receiver) = mpsc::sync_channel(1);
    let (clock_release_sender, clock_release_receiver) = mpsc::sync_channel(1);
    let executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![Arc::new(GateHandler {
            entered: entered_sender,
            release: Mutex::new(release_receiver),
            invocations: Arc::clone(&invocations),
        })],
        ExecutorFixture::config(1),
        Arc::new(BlockingCheckpointClock {
            calls: AtomicUsize::new(0),
            blocked: clock_blocked_sender,
            release: Mutex::new(clock_release_receiver),
        }),
    )
    .expect("start executor");

    executor
        .submit(&job.job_id)
        .expect("submit progressing job");
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("handler enters first checkpoint"),
        1
    );
    release_sender.send(()).expect("start checkpoint write");
    clock_blocked_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("checkpoint blocks after reading running revision");
    let running = fixture.store().get(&job.job_id).expect("running snapshot");
    let external = fixture
        .store()
        .update_progress(&JobProgressUpdateRequest {
            correlation_id: job.correlation_id.clone(),
            current: 0,
            expected_revision: running.revision,
            job_id: job.job_id.clone(),
            message: "Progress eksternal".to_owned(),
            phase: "external.work".to_owned(),
            total: Some(2),
            unit: Some("step".to_owned()),
        })
        .expect("external progress wins first CAS");
    clock_release_sender
        .send(())
        .expect("release stale progress write");
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("handler continues after one retry"),
        2
    );
    let reconciled = fixture
        .store()
        .get(&job.job_id)
        .expect("reconciled handler progress");
    assert_eq!(reconciled.status, "RUNNING");
    assert_eq!(reconciled.progress_current, 1);
    assert_eq!(reconciled.progress_phase.as_deref(), Some("mock.work"));
    assert_eq!(reconciled.revision, external.revision + 1);

    release_sender.send(()).expect("release second checkpoint");
    assert!(executor.core.queue.close_and_drain().is_empty());
    executor.workers[0]
        .completed
        .recv_timeout(Duration::from_secs(2))
        .expect("worker completes after reconciled progress");
    let terminal = fixture.store().get(&job.job_id).expect("succeeded job");
    assert_eq!(terminal.status, "SUCCEEDED");
    assert_eq!(terminal.progress_current, 2);
    assert_eq!(terminal.revision, reconciled.revision + 2);
    assert_eq!(terminal.revision, external.revision + 3);
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
}

#[test]
fn shutdown_drains_pending_jobs_rejects_submissions_and_joins_active_worker() {
    let mut fixture = ExecutorFixture::new("shutdown-drain");
    let active = fixture.enqueue("shutdown.cooperative");
    let pending = fixture.enqueue("shutdown.cooperative");
    let late = fixture.enqueue("shutdown.cooperative");
    let invocations = Arc::new(AtomicUsize::new(0));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let mut executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![Arc::new(ShutdownGateHandler {
            kind: "shutdown.cooperative",
            entered: entered_sender,
            release: Mutex::new(release_receiver),
            invocations: Arc::clone(&invocations),
        })],
        ExecutorFixture::config(2),
        Arc::new(SystemExecutorClock),
    )
    .expect("start executor");

    executor.submit(&active.job_id).expect("submit active job");
    entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("active handler entered");
    executor
        .submit(&pending.job_id)
        .expect("submit pending job");

    let (closed_sender, closed_receiver) = mpsc::sync_channel(1);
    executor
        .set_shutdown_observer_for_test(closed_sender)
        .expect("install shutdown observer");
    let releaser = std::thread::spawn(move || {
        closed_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("shutdown closed queue");
        release_sender.send(()).expect("release active handler");
    });

    executor.shutdown().expect("cooperative shutdown");
    releaser.join().expect("shutdown releaser");

    assert!(matches!(
        executor.submit(&late.job_id),
        Err(JobExecutorError::ShuttingDown)
    ));
    assert_eq!(
        fixture
            .store()
            .get(&active.job_id)
            .expect("active job")
            .status,
        "SUCCEEDED"
    );
    assert_eq!(
        fixture.store().get(&pending.job_id).expect("pending job"),
        pending
    );
    assert!(!executor
        .core
        .admitted
        .lock()
        .expect("admission set")
        .contains(&pending.job_id));
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
}

#[test]
fn shutdown_timeout_retains_worker_for_successful_retry() {
    let mut fixture = ExecutorFixture::new("shutdown-retry");
    let job = fixture.enqueue("shutdown.retry");
    let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let mut config = ExecutorFixture::config(1);
    config.shutdown_timeout = Duration::from_millis(250);
    let mut executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![Arc::new(ShutdownGateHandler {
            kind: "shutdown.retry",
            entered: entered_sender,
            release: Mutex::new(release_receiver),
            invocations: Arc::new(AtomicUsize::new(0)),
        })],
        config,
        Arc::new(SystemExecutorClock),
    )
    .expect("start executor");

    executor.submit(&job.job_id).expect("submit gated job");
    entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("gated handler entered");
    assert!(matches!(
        executor.shutdown(),
        Err(JobExecutorError::ShutdownTimeout)
    ));
    assert!(matches!(
        executor.submit(&job.job_id),
        Err(JobExecutorError::ShuttingDown)
    ));

    release_sender.send(()).expect("release gated handler");
    executor.shutdown().expect("retry joins completed worker");
    assert_eq!(
        fixture
            .store()
            .get(&job.job_id)
            .expect("terminal job")
            .status,
        "SUCCEEDED"
    );
}

#[test]
fn worker_completion_notification_is_not_worker_exit_proof() {
    let fixture = ExecutorFixture::new("worker-exit-proof");
    let (exit_gate_entered_sender, exit_gate_entered_receiver) = mpsc::sync_channel(1);
    let (exit_gate_release_sender, exit_gate_release_receiver) = mpsc::sync_channel(1);
    let exit_gate_release_receiver = Mutex::new(exit_gate_release_receiver);
    let mut config = ExecutorFixture::config(1);
    config.shutdown_timeout = Duration::from_millis(100);
    let mut executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        Vec::new(),
        config,
        Arc::new(SystemExecutorClock),
    )
    .expect("start executor");
    executor
        .set_worker_exit_gate_for_test(Arc::new(move || {
            exit_gate_entered_sender
                .send(())
                .expect("signal worker exit gate");
            exit_gate_release_receiver
                .lock()
                .expect("worker exit gate receiver")
                .recv_timeout(Duration::from_secs(2))
                .expect("release worker exit gate");
        }))
        .expect("install worker exit gate");

    assert!(matches!(
        executor.shutdown(),
        Err(JobExecutorError::ShutdownTimeout)
    ));
    exit_gate_entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("worker reached gate after completion notification");
    exit_gate_release_sender
        .send(())
        .expect("release worker exit gate");
    executor.shutdown().expect("retry joins exited worker");
}

#[test]
fn reaper_completion_notification_is_not_reaper_exit_proof() {
    let fixture = ExecutorFixture::new("reaper-exit-proof");
    let (exit_gate_entered_sender, exit_gate_entered_receiver) = mpsc::sync_channel(1);
    let (exit_gate_release_sender, exit_gate_release_receiver) = mpsc::sync_channel(1);
    let exit_gate_release_receiver = Mutex::new(exit_gate_release_receiver);
    let mut config = ExecutorFixture::config(1);
    config.shutdown_timeout = Duration::from_millis(100);
    let mut executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        Vec::new(),
        config,
        Arc::new(SystemExecutorClock),
    )
    .expect("start executor");
    executor
        .set_reaper_exit_gate_for_test(Arc::new(move || {
            exit_gate_entered_sender
                .send(())
                .expect("signal reaper exit gate");
            exit_gate_release_receiver
                .lock()
                .expect("reaper exit gate receiver")
                .recv_timeout(Duration::from_secs(2))
                .expect("release reaper exit gate");
        }))
        .expect("install reaper exit gate");

    assert!(matches!(
        executor.shutdown(),
        Err(JobExecutorError::ShutdownTimeout)
    ));
    exit_gate_entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("reaper reached gate after completion notification");
    exit_gate_release_sender
        .send(())
        .expect("release reaper exit gate");
    executor.shutdown().expect("retry joins exited reaper");
}

#[test]
fn reaper_command_errors_never_own_or_drop_worker_handles() {
    let slot = WorkerSlot::new();
    let (worker_done_sender, worker_done_receiver) = mpsc::sync_channel(1);
    slot.install(std::thread::spawn(move || {
        worker_done_sender.send(()).expect("signal worker body");
    }));
    worker_done_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("worker body completed");

    let (command_sender, command_receiver) = mpsc::sync_channel(1);
    command_sender
        .try_send(ReaperCommand::Stop)
        .expect("fill command channel");
    assert!(matches!(
        command_sender.try_send(ReaperCommand::Stop),
        Err(mpsc::TrySendError::Full(ReaperCommand::Stop))
    ));
    assert!(slot.has_handle());
    drop(command_receiver);
    assert!(matches!(
        command_sender.try_send(ReaperCommand::Stop),
        Err(mpsc::TrySendError::Disconnected(ReaperCommand::Stop))
    ));
    assert!(slot.has_handle());

    let worker = slot.take().expect("worker handle remains in slot");
    let exit_deadline = std::time::Instant::now()
        .checked_add(Duration::from_secs(1))
        .expect("bounded worker exit deadline");
    assert!(wait_for_join_exit(Some(&worker), exit_deadline));
    worker.join().expect("join retained worker");
}

#[test]
fn drop_after_timeout_disconnects_reaper_to_join_shared_worker_slot() {
    let mut fixture = ExecutorFixture::new("shutdown-drop-reaper");
    let job = fixture.enqueue("shutdown.drop");
    let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let mut config = ExecutorFixture::config(1);
    config.shutdown_timeout = Duration::from_millis(250);
    let mut executor = JobExecutor::new(
        Arc::clone(fixture.store()),
        vec![Arc::new(ShutdownGateHandler {
            kind: "shutdown.drop",
            entered: entered_sender,
            release: Mutex::new(release_receiver),
            invocations: Arc::new(AtomicUsize::new(0)),
        })],
        config,
        Arc::new(SystemExecutorClock),
    )
    .expect("start executor");

    executor.submit(&job.job_id).expect("submit gated job");
    entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("gated handler entered");
    assert!(matches!(
        executor.shutdown(),
        Err(JobExecutorError::ShutdownTimeout)
    ));
    let reaper_completed = executor
        .take_reaper_completion_for_test()
        .expect("reaper completion receiver");
    let (drop_sender, drop_receiver) = mpsc::sync_channel(1);
    let dropper = std::thread::spawn(move || {
        drop(executor);
        drop_sender.send(()).expect("signal bounded drop");
    });

    drop_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("drop returns within configured bounded attempt");
    release_sender.send(()).expect("release adopted worker");
    reaper_completed
        .recv_timeout(Duration::from_secs(2))
        .expect("reaper joined adopted worker");
    dropper.join().expect("dropper thread");
    assert_eq!(
        fixture
            .store()
            .get(&job.job_id)
            .expect("terminal job")
            .status,
        "SUCCEEDED"
    );
}

#[test]
fn restart_recovery_fails_active_jobs_once_and_preserves_queued_job() {
    let mut fixture = ExecutorFixture::new("restart-recovery");
    let running_job = fixture.enqueue("passive.kind");
    let cancelling_job = fixture.enqueue("passive.kind");
    let queued_job = fixture.enqueue("passive.kind");
    let running = fixture
        .store()
        .start(&JobTransitionRequest {
            correlation_id: running_job.correlation_id.clone(),
            expected_revision: running_job.revision,
            job_id: running_job.job_id.clone(),
        })
        .expect("start running fixture");
    let started_for_cancellation = fixture
        .store()
        .start(&JobTransitionRequest {
            correlation_id: cancelling_job.correlation_id.clone(),
            expected_revision: cancelling_job.revision,
            job_id: cancelling_job.job_id.clone(),
        })
        .expect("start cancelling fixture");
    let cancelling = fixture
        .store()
        .request_cancellation(&JobTransitionRequest {
            correlation_id: cancelling_job.correlation_id.clone(),
            expected_revision: started_for_cancellation.revision,
            job_id: cancelling_job.job_id.clone(),
        })
        .expect("request fixture cancellation");

    drop(fixture.store.take());
    fixture.store = Some(Arc::new(
        JobStore::open(&fixture.project_path).expect("reopen job store after interruption"),
    ));
    let recovered = fixture
        .store()
        .recover_interrupted(RECOVERY_CORRELATION_ID)
        .expect("recover interrupted jobs");
    assert_eq!(recovered.len(), 2);

    let recovered_running = fixture
        .store()
        .get(&running.job_id)
        .expect("recovered running job");
    let recovered_cancelling = fixture
        .store()
        .get(&cancelling.job_id)
        .expect("recovered cancelling job");
    for descriptor in [&recovered_running, &recovered_cancelling] {
        assert_eq!(descriptor.status, "FAILED");
        assert_eq!(descriptor.error_code.as_deref(), Some("INTERRUPTED"));
        assert_eq!(descriptor.error_retriable, Some(true));
        assert_job_audit_chain(&fixture, descriptor);
    }
    assert_eq!(
        fixture
            .store()
            .get(&queued_job.job_id)
            .expect("queued job unchanged"),
        queued_job
    );
    let history_before_retry = (
        job_history_counts(&fixture.project_path, &running.job_id),
        job_history_counts(&fixture.project_path, &cancelling.job_id),
        job_history_counts(&fixture.project_path, &queued_job.job_id),
    );
    assert!(fixture
        .store()
        .recover_interrupted(RECOVERY_CORRELATION_ID)
        .expect("repeat recovery")
        .is_empty());
    assert_eq!(
        history_before_retry,
        (
            job_history_counts(&fixture.project_path, &running.job_id),
            job_history_counts(&fixture.project_path, &cancelling.job_id),
            job_history_counts(&fixture.project_path, &queued_job.job_id),
        )
    );
}

#[test]
fn sixteen_simultaneous_duplicate_submissions_admit_and_invoke_exactly_once() {
    const SUBMITTERS: usize = 16;

    let mut fixture = ExecutorFixture::new("duplicate-stress");
    let job = fixture.enqueue("duplicate.gate");
    let invocations = Arc::new(AtomicUsize::new(0));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let executor = Arc::new(
        JobExecutor::new(
            Arc::clone(fixture.store()),
            vec![Arc::new(ShutdownGateHandler {
                kind: "duplicate.gate",
                entered: entered_sender,
                release: Mutex::new(release_receiver),
                invocations: Arc::clone(&invocations),
            })],
            ExecutorFixture::config(1),
            Arc::new(SystemExecutorClock),
        )
        .expect("start executor"),
    );
    let start = Arc::new(Barrier::new(SUBMITTERS + 1));
    let (result_sender, result_receiver) = mpsc::sync_channel(SUBMITTERS);
    let mut submitters = Vec::with_capacity(SUBMITTERS);
    for _ in 0..SUBMITTERS {
        let executor = Arc::clone(&executor);
        let start = Arc::clone(&start);
        let result_sender = result_sender.clone();
        let job_id = job.job_id.clone();
        submitters.push(std::thread::spawn(move || {
            start.wait();
            result_sender
                .send(executor.submit(&job_id))
                .expect("send duplicate submit result");
        }));
    }
    drop(result_sender);
    start.wait();

    let mut accepted = 0;
    let mut duplicates = 0;
    for _ in 0..SUBMITTERS {
        match result_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("duplicate submit result")
        {
            Ok(()) => accepted += 1,
            Err(JobExecutorError::AlreadySubmitted) => duplicates += 1,
            Err(error) => panic!("unexpected duplicate submit result: {error}"),
        }
    }
    for submitter in submitters {
        submitter.join().expect("duplicate submitter");
    }
    assert_eq!(accepted, 1);
    assert_eq!(duplicates, SUBMITTERS - 1);
    entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("admitted handler invoked");
    assert_eq!(invocations.load(Ordering::SeqCst), 1);

    release_sender.send(()).expect("release admitted handler");
    let Ok(mut executor) = Arc::try_unwrap(executor) else {
        panic!("all submitter executor references released");
    };
    executor.shutdown().expect("join stress worker");
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
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
