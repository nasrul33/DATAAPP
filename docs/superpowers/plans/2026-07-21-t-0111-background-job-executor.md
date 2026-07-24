# T-0111 Background Job Executor Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a bounded, project-scoped Rust executor that runs a deterministic mock long job asynchronously while persisting progress, cancellation, failures, audit history, resource preflight, shutdown, and restart recovery through the existing T-0110 `JobStore`.

**Architecture:** `JobStore` remains the only durable lifecycle authority. A new `job_executor` module owns volatile admission, a standard-library bounded queue, worker threads, handler dispatch, explicit resource reservations, and shutdown; handlers receive an `ExecutionContext` instead of direct store access. Queue, resource, and orchestration code live in separate focused files, with deterministic tests using an injected clock and checkpoint gates.

**Tech Stack:** Rust 2021 workspace, standard-library threads/synchronization, existing `rusqlite`, `time`, generated job contracts, Cargo/Clippy test gates. No runtime dependency is added.

## Global Constraints

- Preserve T-0110 transition, CAS, append-only event, audit, pinned metadata, and recovery semantics.
- Do not execute Python, shell commands, paths, source rows, dataset values, or arbitrary payloads.
- Do not add Tauri commands/events, frontend UI, retry behavior, or cross-process coordination.
- Every queue, string, progress update, resource estimate, worker set, and wait is bounded.
- Create the executor-private worker reaper before spawning workers; construction fails cleanly if the reaper cannot start.
- Executor limits are explicit configuration; do not hardcode environment-dependent thresholds or batch sizes.
- Handler panic text, raw database errors, absolute paths, source values, and secrets must never cross the safe error boundary.
- Use strict Rust lints; no `unsafe`, `unwrap`, or unjustified `allow` attributes in production code.
- Keep Python at 3.12, Tauri at 2.x, and all existing TypeScript strict settings unchanged.
- Update `docs/CONTEXT_PACK.md` after implementation and run the complete `AGENTS.md` gate matrix.

---

## File map

| Path | Responsibility |
|---|---|
| `crates/app-core/src/job_executor/mod.rs` | Public executor API, handler/context contracts, registry, worker lifecycle, state reconciliation, safe errors |
| `crates/app-core/src/job_executor/queue.rs` | Bounded FIFO, non-blocking push, blocking worker pop, close-and-drain |
| `crates/app-core/src/job_executor/resource.rs` | Duration class, estimates, explicit budgets, checked aggregate reservations, RAII release |
| `crates/app-core/src/job_executor/tests.rs` | Project fixture, deterministic clock/gates/handlers, lifecycle and stress tests |
| `crates/app-core/src/job.rs` | Expose only crate-private status parsing and timestamped store methods needed by the executor |
| `crates/app-core/src/lib.rs` | Register module and re-export the supported executor surface |
| `docs/IMPLEMENTATION_PLAN.md` | Add T-0111 scope and acceptance |
| `data-contracts/IPC_CONTRACTS.md` | Document executor-only runtime semantics and explicit exclusions |
| `artifacts/THREAT_MODEL.md` | Add handler, panic, resource, and shutdown trust boundaries |
| `README.md` | Record verified executor test command if it adds developer value |
| `docs/CONTEXT_PACK.md` | Record decisions, evidence, residual risks, and completion status |

---

### Task 1: Checked resource contracts and RAII ledger

**Files:**
- Create: `crates/app-core/src/job_executor/resource.rs`
- Create: `crates/app-core/src/job_executor/mod.rs`
- Modify: `crates/app-core/src/lib.rs`
- Test: `crates/app-core/src/job_executor/resource.rs`

**Interfaces:**
- Produces: `DurationClass`, `ResourceEstimate`, `ResourceBudget`, `ResourceLedger`, `ResourceReservation`, and `ResourceError`.
- Later tasks consume `ResourceLedger::reserve(estimate)` before transitioning a job to `RUNNING`.

- [ ] **Step 1: Add failing resource tests**

Add the module declarations and these tests at the bottom of `resource.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{DurationClass, ResourceBudget, ResourceError, ResourceEstimate, ResourceLedger};

    fn estimate(memory_bytes: u64, disk_bytes: u64, duration: DurationClass) -> ResourceEstimate {
        ResourceEstimate { memory_bytes, disk_bytes, duration }
    }

    #[test]
    fn reservations_are_checked_aggregated_and_released() {
        let ledger = ResourceLedger::new(ResourceBudget {
            memory_bytes: 100,
            disk_bytes: 200,
            max_duration: DurationClass::Medium,
        })
        .expect("valid budget");
        let first = ledger
            .reserve(estimate(60, 80, DurationClass::Short))
            .expect("first reservation");
        assert!(matches!(
            ledger.reserve(estimate(50, 10, DurationClass::Short)),
            Err(ResourceError::MemoryExceeded)
        ));
        drop(first);
        assert!(ledger
            .reserve(estimate(100, 200, DurationClass::Medium))
            .is_ok());
    }

    #[test]
    fn invalid_or_overlong_work_is_rejected_without_reservation() {
        assert!(matches!(
            ResourceLedger::new(ResourceBudget {
                memory_bytes: 0,
                disk_bytes: 1,
                max_duration: DurationClass::Long,
            }),
            Err(ResourceError::InvalidBudget)
        ));
        let ledger = ResourceLedger::new(ResourceBudget {
            memory_bytes: 10,
            disk_bytes: 10,
            max_duration: DurationClass::Short,
        })
        .expect("valid budget");
        assert!(matches!(
            ledger.reserve(estimate(1, 1, DurationClass::Long)),
            Err(ResourceError::DurationExceeded)
        ));
        assert_eq!(ledger.reserved_for_test(), (0, 0));
    }
}
```

- [ ] **Step 2: Run the resource tests and confirm red**

Run: `cargo test -p teratai-app-core job_executor::resource::tests --locked`

Expected: FAIL because the resource types and methods are not defined.

- [ ] **Step 3: Implement the complete resource ledger**

Implement these exact public contracts in `resource.rs`:

```rust
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DurationClass {
    Short,
    Medium,
    Long,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceEstimate {
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub duration: DurationClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceBudget {
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub max_duration: DurationClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceError {
    InvalidBudget,
    MemoryExceeded,
    DiskExceeded,
    DurationExceeded,
    ArithmeticOverflow,
    Poisoned,
}

#[derive(Debug, Default)]
struct Reserved {
    memory_bytes: u64,
    disk_bytes: u64,
}

#[derive(Debug)]
struct LedgerInner {
    budget: ResourceBudget,
    reserved: Mutex<Reserved>,
}

#[derive(Debug, Clone)]
pub struct ResourceLedger {
    inner: Arc<LedgerInner>,
}

#[derive(Debug)]
pub struct ResourceReservation {
    inner: Arc<LedgerInner>,
    estimate: ResourceEstimate,
}
```

`ResourceLedger::new` rejects zero byte budgets. `reserve` checks duration first, uses `checked_add` for both counters, compares aggregate values to the budget, commits both counters under one mutex, and returns `ResourceReservation`. `Drop for ResourceReservation` subtracts the exact estimate while holding the mutex. `reserved_for_test` is `#[cfg(test)]` and returns both counters.

- [ ] **Step 4: Verify resource behavior and lints**

Run: `cargo test -p teratai-app-core job_executor::resource::tests --locked`

Expected: 2 passed, 0 failed.

Run: `cargo clippy -p teratai-app-core --all-targets --locked -- -D warnings`

Expected: exit 0 with no warnings.

- [ ] **Step 5: Commit the resource primitive**

```powershell
git add crates/app-core/src/job_executor/resource.rs crates/app-core/src/job_executor/mod.rs crates/app-core/src/lib.rs
git commit -m "feat(job): add executor resource ledger"
```

---

### Task 2: Bounded closeable worker queue

**Files:**
- Create: `crates/app-core/src/job_executor/queue.rs`
- Modify: `crates/app-core/src/job_executor/mod.rs`
- Test: `crates/app-core/src/job_executor/queue.rs`

**Interfaces:**
- Produces: crate-private `BoundedQueue<T>`, `PushError<T>`, `try_push`, `pop`, and `close_and_drain`.
- Later tasks share one queue through `Arc<BoundedQueue<String>>`.

- [ ] **Step 1: Write deterministic queue tests**

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{BoundedQueue, PushError};

    #[test]
    fn push_is_fifo_and_full_is_non_blocking() {
        let queue = BoundedQueue::new(2).expect("valid capacity");
        queue.try_push("a").expect("first item");
        queue.try_push("b").expect("second item");
        assert!(matches!(queue.try_push("c"), Err(PushError::Full("c"))));
        assert_eq!(queue.pop(), Some("a"));
        assert_eq!(queue.pop(), Some("b"));
    }

    #[test]
    fn close_drains_pending_items_and_wakes_waiters() {
        let queue = Arc::new(BoundedQueue::<&str>::new(2).expect("valid capacity"));
        let waiting = Arc::clone(&queue);
        let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            result_sender.send(waiting.pop()).expect("send pop result");
        });
        assert!(queue.close_and_drain().is_empty());
        assert_eq!(
            result_receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("closed queue wakes worker"),
            None
        );
        worker.join().expect("worker");

        let pending = BoundedQueue::new(2).expect("valid pending queue");
        pending.try_push("not-started").expect("pending item");
        assert_eq!(pending.close_and_drain(), vec!["not-started"]);
        assert_eq!(queue.pop(), None);
        assert!(matches!(
            queue.try_push("late"),
            Err(PushError::Closed("late"))
        ));
    }
}
```

- [ ] **Step 2: Run the queue tests and confirm red**

Run: `cargo test -p teratai-app-core job_executor::queue::tests --locked`

Expected: FAIL because `BoundedQueue` and `PushError` are missing.

- [ ] **Step 3: Implement the queue with `Mutex<VecDeque<T>>` and `Condvar`**

Use this state shape:

```rust
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PushError<T> {
    Full(T),
    Closed(T),
    Poisoned(T),
}

#[derive(Debug)]
struct QueueState<T> {
    items: VecDeque<T>,
    closed: bool,
}

#[derive(Debug)]
pub(super) struct BoundedQueue<T> {
    capacity: usize,
    state: Mutex<QueueState<T>>,
    available: Condvar,
}
```

`new(0)` returns `None`; positive capacity uses `VecDeque::try_reserve_exact` and returns `None` on allocation failure. `try_push` never waits. `pop` waits only while open and empty, handles spurious wakeups in a loop, and returns `None` after close. `close_and_drain` sets `closed`, drains all pending items in FIFO order, notifies all workers, and is idempotent.

- [ ] **Step 4: Verify queue tests, format, and Clippy**

Run: `cargo test -p teratai-app-core job_executor::queue::tests --locked`

Expected: 2 passed, 0 failed.

Run: `cargo fmt --check && cargo clippy -p teratai-app-core --all-targets --locked -- -D warnings`

Expected: exit 0.

- [ ] **Step 5: Commit the queue**

```powershell
git add crates/app-core/src/job_executor/queue.rs crates/app-core/src/job_executor/mod.rs
git commit -m "feat(job): add bounded executor queue"
```

---

### Task 3: Executor contracts, validated core, registry, and admission

**Files:**
- Modify: `crates/app-core/src/job_executor/mod.rs`
- Create: `crates/app-core/src/job_executor/tests.rs`
- Modify: `crates/app-core/src/job.rs`
- Modify: `crates/app-core/src/lib.rs`
- Test: `crates/app-core/src/job_executor/tests.rs`

**Interfaces:**
- Consumes: `BoundedQueue`, `ResourceLedger`, T-0110 `JobStore`, and generated job requests.
- Produces: `JobExecutorConfig`, `ExecutorClock`, `SystemExecutorClock`, `JobHandler`, `JobHandlerError`, `HandlerOutcome`, `JobProgress`, `CheckpointDecision`, `JobExecutorError`, `JobExecutorErrorKind`, and crate-private `ExecutorCore`.
- Task 4 wraps the validated core in the public `JobExecutor` and starts workers; Task 3 must not export an executor that accepts and discards work.

- [ ] **Step 1: Add fixture and failing configuration/admission tests**

Create `tests.rs` with an `ExecutorFixture` that:

1. creates a unique absolute `*.teratai` path under `std::env::temp_dir()`;
2. calls `ProjectService::create` with fixed valid lowercase UUID-v7 identifiers;
3. calls `ProjectService::upgrade` with a distinct UUID-v7 correlation ID;
4. opens `Arc<JobStore>`;
5. removes the project only after every executor/store handle is dropped.

Add tests asserting:

```rust
#[test]
fn invalid_config_starts_no_workers() {
    let fixture = ExecutorFixture::new("invalid-config");
    let result = JobExecutor::new(
        Arc::clone(&fixture.store),
        Vec::new(),
        JobExecutorConfig {
            worker_count: 0,
            queue_capacity: 1,
            resource_budget: fixture.budget(),
            shutdown_timeout: Duration::from_secs(1),
        },
        Arc::new(FixedClock::default()),
    );
    assert!(matches!(result, Err(JobExecutorError::InvalidConfiguration)));
}

#[test]
fn unknown_kind_does_not_mutate_job() {
    let fixture = ExecutorFixture::new("admission");
    let queued = fixture.enqueue("known.kind");
    let core = fixture.executor_core(Vec::new());
    assert!(matches!(
        core.claim_and_queue(&queued.job_id),
        Err(JobExecutorError::HandlerNotFound)
    ));
    assert_eq!(fixture.store.get(&queued.job_id).expect("job"), queued);
}
```

The fixture must use channels/barriers with timeouts, not scheduler sleeps.

- [ ] **Step 2: Run admission tests and confirm red**

Run: `cargo test -p teratai-app-core job_executor::tests --locked`

Expected: FAIL because executor contracts do not exist.

- [ ] **Step 3: Define the supported executor surface**

Add these exact signatures to `mod.rs`:

```rust
#[derive(Debug, Clone)]
pub struct JobExecutorConfig {
    pub worker_count: usize,
    pub queue_capacity: usize,
    pub resource_budget: ResourceBudget,
    pub shutdown_timeout: std::time::Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockError;

pub trait ExecutorClock: Send + Sync {
    fn now(&self) -> Result<String, ClockError>;
}

#[derive(Debug, Default)]
pub struct SystemExecutorClock;

pub trait JobHandler: Send + Sync {
    fn kind(&self) -> &'static str;
    fn estimate(&self) -> ResourceEstimate;
    fn run(
        &self,
        context: &mut ExecutionContext<'_>,
    ) -> Result<HandlerOutcome, JobHandlerError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerOutcome {
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobProgress {
    pub current: i64,
    pub total: Option<i64>,
    pub unit: Option<String>,
    pub phase: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointDecision {
    Continue,
    Cancelled,
}

pub struct ExecutionContext<'a> {
    store: &'a JobStore,
    descriptor: JobDescriptor,
    clock: &'a dyn ExecutorClock,
}

impl ExecutionContext<'_> {
    pub fn checkpoint(
        &mut self,
        progress: JobProgress,
    ) -> Result<CheckpointDecision, JobExecutorError>;
    pub fn cancellation_requested(&mut self) -> Result<bool, JobExecutorError>;
    pub fn job_id(&self) -> &str;
    pub fn project_id(&self) -> &str;
    pub fn correlation_id(&self) -> &str;
}
```

`JobHandlerError::new(code, message, retriable)` validates code pattern `[A-Z][A-Z0-9_]*` at `1..=120` bytes, trimmed control-free message length `1..=500` bytes, and stores no source error. `JobExecutorError::kind()` maps every design error to a stable enum and `Display` emits only fixed safe text.

- [ ] **Step 4: Open the minimum crate-private JobStore adapter**

In `job.rs`, change only these existing items to `pub(crate)`:

```rust
pub(crate) enum JobStatus { Queued, Running, Succeeded, Failed, Cancelling, Cancelled }

impl JobStatus {
    pub(crate) fn parse(value: &str) -> Result<Self, JobError>;
    pub(crate) const fn is_terminal(self) -> bool;
}

pub(crate) fn start_at(
    &self,
    request: &JobTransitionRequest,
    timestamp: &str,
) -> Result<JobDescriptor, JobError>;

pub(crate) fn succeed_at(
    &self,
    request: &JobTransitionRequest,
    timestamp: &str,
) -> Result<JobDescriptor, JobError>;

pub(crate) fn fail_at(
    &self,
    request: &JobFailureRequest,
    timestamp: &str,
) -> Result<JobDescriptor, JobError>;

pub(crate) fn update_progress_at(
    &self,
    request: &JobProgressUpdateRequest,
    timestamp: &str,
) -> Result<JobDescriptor, JobError>;

pub(crate) fn complete_cancellation_at(
    &self,
    request: &JobTransitionRequest,
    timestamp: &str,
) -> Result<JobDescriptor, JobError>;
```

Do not make timestamped mutation methods part of the public crate API. Export only supported executor types from `lib.rs`.

- [ ] **Step 5: Implement validated construction and non-blocking admission**

`ExecutorCore::new` must:

- reject zero workers, zero queue capacity, zero shutdown timeout, and worker counts above `available_parallelism()`;
- reject duplicate or invalid handler kinds before spawning any thread;
- construct the queue/ledger before workers;
- retain an `Arc<Mutex<HashSet<String>>>` admission set.

`ExecutorCore::claim_and_queue(job_id)` must validate through `JobStore::get`, require status `QUEUED`, find the handler before claiming admission, atomically insert the ID, call `try_push`, and remove the ID on every push failure. Map full to `QueueFull`, closed to `ShuttingDown`, and poisoned synchronization to `PersistenceFailed` without including raw details. Keep `ExecutorCore` crate-private and do not spawn workers yet.

Add a `PassiveHandler` fixture whose `run` returns `HandlerOutcome::Completed` but is never invoked by `ExecutorCore`. With queue capacity one, assert:

```rust
let first = fixture.enqueue("passive.kind");
let second = fixture.enqueue("passive.kind");
let core = fixture.executor_core(vec![Arc::new(PassiveHandler)]);
core.claim_and_queue(&first.job_id).expect("first admission");
assert!(matches!(
    core.claim_and_queue(&first.job_id),
    Err(JobExecutorError::AlreadySubmitted)
));
assert!(matches!(
    core.claim_and_queue(&second.job_id),
    Err(JobExecutorError::QueueFull)
));
assert_eq!(fixture.store.get(&second.job_id).expect("second job"), second);
```

- [ ] **Step 6: Verify admission behavior**

Run: `cargo test -p teratai-app-core job_executor::tests --locked`

Expected: configuration and core admission tests pass; no public executor or worker execution exists yet.

Run: `cargo clippy -p teratai-app-core --all-targets --locked -- -D warnings`

Expected: exit 0.

- [ ] **Step 7: Commit executor contracts and admission**

```powershell
git add crates/app-core/src/job_executor crates/app-core/src/job.rs crates/app-core/src/lib.rs
git commit -m "feat(job): add executor admission boundary"
```

---

### Task 4: Public executor, worker success path, and persistent progress

**Files:**
- Modify: `crates/app-core/src/job_executor/mod.rs`
- Modify: `crates/app-core/src/job_executor/tests.rs`
- Test: `crates/app-core/src/job_executor/tests.rs`

**Interfaces:**
- Consumes: executor admission, crate-private timestamped `JobStore` methods, `ExecutionContext`, and resource reservation.
- Produces: public `JobExecutor`, `JobExecutor::new`, `JobExecutor::submit`, worker loop, progress checkpoints, and terminal success.

- [ ] **Step 1: Add a deterministic mock success handler and failing test**

Define `GateHandler` in `tests.rs` with channel-driven checkpoints and an `AtomicUsize` invocation count. Its `run` method performs exactly two progress checkpoints:

```rust
fn run(
    &self,
    context: &mut ExecutionContext<'_>,
) -> Result<HandlerOutcome, JobHandlerError> {
    self.invocations.fetch_add(1, Ordering::SeqCst);
    for current in [1, 2] {
        self.entered.send(current).expect("signal checkpoint");
        self.release
            .lock()
            .expect("release receiver")
            .recv_timeout(Duration::from_secs(2))
            .expect("release checkpoint");
        if context.checkpoint(JobProgress {
            current,
            total: Some(2),
            unit: Some("step".to_owned()),
            phase: "mock.work".to_owned(),
            message: format!("Langkah {current} dari 2"),
        })
        .expect("persist checkpoint") == CheckpointDecision::Cancelled
        {
            return Ok(HandlerOutcome::Cancelled);
        }
    }
    Ok(HandlerOutcome::Completed)
}
```

Test that the first signal observes persistent `RUNNING`, each release produces monotonic progress, and completion produces `SUCCEEDED`, revision growth, one handler invocation, and reopen-safe state.

- [ ] **Step 2: Run the success test and confirm red**

Run: `cargo test -p teratai-app-core job_executor::tests::mock_job_persists_progress_and_succeeds --locked`

Expected: FAIL because workers do not execute handlers.

- [ ] **Step 3: Implement worker ownership and execution context**

`JobExecutor::new` first creates `ExecutorCore`, then starts workers. `JobExecutor::submit` delegates to `ExecutorCore::claim_and_queue`; no other public submission path exists. For each configured worker, spawn one named thread and retain:

```rust
struct WorkerHandle {
    join: Option<std::thread::JoinHandle<()>>,
    completed: std::sync::mpsc::Receiver<()>,
}
```

The worker loop calls `queue.pop()`, creates an RAII admission guard, re-reads the job, skips non-`QUEUED` snapshots, reserves the handler estimate, transitions through `start_at`, and runs the handler. `ExecutionContext::checkpoint` re-reads state, returns `Cancelled` for `CANCELLING`, otherwise requires `RUNNING` and calls `update_progress_at` using its clock timestamp and current revision.

After `HandlerOutcome::Completed`, re-read once: complete cancellation if state is `CANCELLING`, otherwise call `succeed_at` only from `RUNNING`. Release the resource and admission guards after the terminal decision.

- [ ] **Step 4: Verify success, persistence, and existing job tests**

Run: `cargo test -p teratai-app-core job_executor::tests::mock_job_persists_progress_and_succeeds --locked`

Expected: 1 passed.

Run: `cargo test -p teratai-app-core job::tests --locked`

Expected: all existing T-0110 tests pass unchanged.

- [ ] **Step 5: Commit the worker success path**

```powershell
git add crates/app-core/src/job_executor
git commit -m "feat(job): execute persisted jobs in background"
```

---

### Task 5: Queued/running cancellation and CAS reconciliation

**Files:**
- Modify: `crates/app-core/src/job_executor/mod.rs`
- Modify: `crates/app-core/src/job_executor/tests.rs`
- Test: `crates/app-core/src/job_executor/tests.rs`

**Interfaces:**
- Consumes: T-0110 `request_cancellation`, executor checkpoints, current descriptor revision.
- Produces: queued cancellation skip, running cancellation completion, and one-time conflict reconciliation.

- [ ] **Step 1: Add failing queued and running cancellation tests**

The queued test holds the only worker in a first job, submits a second job, changes the second snapshot to `CANCELLING`, releases the first job, and asserts the second handler is never invoked and the second job becomes `CANCELLED`.

The running test waits for `GateHandler` checkpoint 1, calls:

```rust
let running = fixture.store.get(&job.job_id).expect("running snapshot");
fixture
    .store
    .request_cancellation(&JobTransitionRequest {
        job_id: job.job_id.clone(),
        correlation_id: job.correlation_id.clone(),
        expected_revision: running.revision,
    })
    .expect("request cancellation");
```

Release the checkpoint and assert terminal `CANCELLED`, no second unit of work, and no duplicate event after repeating cancellation with the current cancelling revision.

- [ ] **Step 2: Run both cancellation tests and confirm red**

Run: `cargo test -p teratai-app-core job_executor::tests --locked`

Expected: at least one FAIL because queued cancellation and revision reconciliation are incomplete.

- [ ] **Step 3: Implement cancellation and one-time conflict reconciliation**

Before preflight, a worker seeing `CANCELLING` calls `complete_cancellation_at`. During execution, `checkpoint` returns `Cancelled` without a progress write. On any `RevisionConflict`, re-read exactly once:

- `CANCELLING`: follow cancellation completion;
- terminal: stop without another write;
- `RUNNING`: retry only the same progress mutation with the new revision;
- all other state: return `PersistenceConflict` and stop.

Do not loop indefinitely and do not create a second cancellation flag.

- [ ] **Step 4: Verify cancellation and transition history**

Run: `cargo test -p teratai-app-core job_executor::tests --locked`

Expected: all executor tests pass.

Run: `cargo test -p teratai-app-core job::tests::cancellation_request_is_idempotent_and_does_not_finish_early --locked`

Expected: 1 passed.

- [ ] **Step 5: Commit cancellation support**

```powershell
git add crates/app-core/src/job_executor
git commit -m "feat(job): add cooperative executor cancellation"
```

---

### Task 6: Preflight failure, typed handler failure, and panic containment

**Files:**
- Modify: `crates/app-core/src/job_executor/mod.rs`
- Modify: `crates/app-core/src/job_executor/tests.rs`
- Test: `crates/app-core/src/job_executor/tests.rs`

**Interfaces:**
- Consumes: `ResourceLedger`, `JobHandlerError`, `JobStore::fail_at`.
- Produces: safe `RESOURCE_LIMIT` and `OPERATION_FAILED` terminal outcomes plus panic isolation.

- [ ] **Step 1: Add failing resource/failure/panic tests**

Add three native handlers:

- `OverBudgetHandler`, whose estimate exceeds the configured memory budget and whose invocation counter must remain zero;
- `FailingHandler`, which returns `JobHandlerError::new("MOCK_FAILED", "Pekerjaan mock gagal.", true)`;
- `PanicHandler`, which panics with a sentinel secret and is followed by a normal job on the same worker.

Assert exact persistent outcomes:

```rust
assert_eq!(preflight.error_code.as_deref(), Some("RESOURCE_LIMIT"));
assert_eq!(failed.error_code.as_deref(), Some("MOCK_FAILED"));
assert_eq!(panicked.error_code.as_deref(), Some("OPERATION_FAILED"));
assert!(!panicked.error_message.as_deref().unwrap_or_default().contains("sentinel"));
assert_eq!(following.status, "SUCCEEDED");
```

- [ ] **Step 2: Run failure tests and confirm red**

Run: `cargo test -p teratai-app-core job_executor::tests --locked`

Expected: FAIL because terminal failure normalization and panic containment are missing.

- [ ] **Step 3: Implement safe failure normalization**

Wrap only the handler call in:

```rust
std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler.run(&mut context)))
```

Map outcomes as follows:

| Outcome | Persistent code | Message source |
|---|---|---|
| resource rejection | `RESOURCE_LIMIT` | fixed Indonesian safe text |
| typed handler error | validated handler code | validated handler message |
| panic | `OPERATION_FAILED` | fixed Indonesian safe text |

Build `JobFailureRequest` from the latest trusted snapshot and call `fail_at`. Never format the panic payload or a `JobError` into persistent text. Ensure both RAII guards drop even if failure persistence itself returns an error.

- [ ] **Step 4: Verify failure isolation and audit atomicity**

Run: `cargo test -p teratai-app-core job_executor::tests --locked`

Expected: all executor tests pass.

Run: `cargo clippy -p teratai-app-core --all-targets --locked -- -D warnings`

Expected: exit 0.

- [ ] **Step 5: Commit safe failure handling**

```powershell
git add crates/app-core/src/job_executor
git commit -m "feat(job): contain executor failures"
```

---

### Task 7: Bounded shutdown, restart recovery, and duplicate stress proof

**Files:**
- Modify: `crates/app-core/src/job_executor/mod.rs`
- Modify: `crates/app-core/src/job_executor/tests.rs`
- Test: `crates/app-core/src/job_executor/tests.rs`

**Interfaces:**
- Consumes: queue close/drain, worker completion receivers, admission set, T-0110 `recover_interrupted`.
- Produces: `JobExecutor::shutdown(&mut self)`, safe `Drop`, retryable timeout state, and exit-gate proof.

- [ ] **Step 1: Add failing shutdown/recovery/stress tests**

Add tests that prove:

1. `shutdown` rejects later submissions, drains pending IDs, releases their admission claims, leaves their snapshots `QUEUED`, and joins a cooperative active worker.
2. A gated non-cooperative handler causes `ShutdownTimeout`; after the test releases its gate, a second `shutdown` joins successfully.
3. Dropping after `ShutdownTimeout` returns within a bounded test deadline, disconnects the payload-free reaper command sender, and the pre-started reaper takes/joins the outstanding handle from its shared slot after the handler gate is released.
4. Dropping an interrupted process fixture and reopening its store lets `recover_interrupted` change active state to `FAILED/INTERRUPTED` once, while queued state remains unchanged.
5. Sixteen simultaneous `submit` calls for one `job_id` yield exactly one success, fifteen `AlreadySubmitted` results, and one handler invocation. Hold the admitted handler at a channel-driven gate until all submitters report; use a start barrier and bounded channels, not timing, as the assertion.

- [ ] **Step 2: Run shutdown/stress tests and confirm red**

Run: `cargo test -p teratai-app-core job_executor::tests --locked`

Expected: FAIL because bounded join/retry and drain cleanup are incomplete.

- [ ] **Step 3: Implement bounded, retryable shutdown**

`shutdown(&mut self)` must:

- atomically mark the executor non-accepting;
- call `close_and_drain` once and remove every drained ID from admission;
- calculate one absolute deadline from the injected shutdown timeout;
- wait on each worker completion receiver only for the remaining duration;
- join only workers known complete;
- retain unjoined handles after timeout so a later `shutdown` can retry;
- stop and join the idle reaper only after every worker handle is joined;
- return success only when every worker and the reaper are joined.

`JobExecutor::new` preallocates one `Arc<WorkerSlot>` per worker, registers clones with a panic-free executor-private `WorkerReaper`, and starts the reaper before worker creation. Every worker installs its `JoinHandle` in its shared slot. The bounded command channel is payload-free and carries only `Stop`; it never owns a worker handle. `Drop` closes/drains, performs only the configured bounded shutdown attempt, then drops the sender and returns without joining the reaper. On disconnection, the existing reaper takes and joins every remaining slot handle asynchronously, signals completion for deterministic tests, and exits. `Full` or `Disconnected` command errors cannot drop handles. Production owners must still call `shutdown`; disconnect-triggered reaper cleanup is the safety fallback, not a successful graceful shutdown.

- [ ] **Step 4: Run executor tests repeatedly to expose flakes**

Run:

```powershell
1..10 | ForEach-Object {
  cargo test -p teratai-app-core job_executor::tests --locked
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
```

Expected: all 10 iterations pass without hang or scheduler-dependent timeout assertion.

- [ ] **Step 5: Run the complete Rust workspace gate**

Run: `cargo fmt --check`

Expected: exit 0.

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`

Expected: exit 0.

Run: `cargo test --workspace --locked`

Expected: every existing and new Rust test passes.

- [ ] **Step 6: Commit shutdown and recovery proof**

```powershell
git add crates/app-core/src/job_executor
git commit -m "feat(job): add bounded executor shutdown"
```

---

### Task 8: Traceability, documentation, and full milestone verification

**Files:**
- Modify: `docs/IMPLEMENTATION_PLAN.md`
- Modify: `data-contracts/IPC_CONTRACTS.md`
- Modify: `artifacts/THREAT_MODEL.md`
- Modify: `README.md`
- Modify: `docs/CONTEXT_PACK.md`
- Test: full repository gate matrix

**Interfaces:**
- Consumes: final executor behavior and verified command results.
- Produces: T-0111 traceability, residual-risk record, and completion evidence.

- [ ] **Step 1: Update implementation and IPC scope truthfully**

Add T-0111 to EPIC-110 with this acceptance statement:

```markdown
| T-0111 | Background job executor core | `crates/app-core` | bounded mock job runs asynchronously; progress/cancellation/failure remain persistent and audit-backed; resource preflight, panic containment, shutdown, and restart recovery are deterministic |
```

In IPC contracts, state explicitly that T-0111 adds no Tauri command/event and no Python engine dispatch. Document in-process exactly-once admission, persistent cancellation observation, safe panic mapping, and explicit configured resource budgets.

- [ ] **Step 2: Update threat model and context pack**

Record these residuals without weakening acceptance:

- exactly-once admission is in-process only;
- non-cooperative native handlers cannot be forcibly killed and surface `ShutdownTimeout`;
- configured budgets are policy ceilings, not physical memory/free-disk discovery;
- panic payloads are discarded and never persisted;
- engine dispatch, platform resource probes, Tauri events, and Job Center remain follow-up scope.

Mark T-0111 complete only after Step 4 passes and include exact test counts from that run.

- [ ] **Step 3: Run frozen dependency synchronization**

Run:

```powershell
$env:CI='true'
$env:UV_CACHE_DIR='D:\DATAAPP\.uv-cache'
$env:PYTEST_ADDOPTS='-p no:cacheprovider'
pnpm install --frozen-lockfile
uv sync --frozen
```

Expected: both commands exit 0 without lockfile changes.

- [ ] **Step 4: Run every required quality gate**

Run each command separately and stop on the first non-zero exit:

```powershell
pnpm lint
pnpm typecheck
pnpm test
pnpm test:e2e
pnpm build
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
uv run ruff check engine
uv run mypy engine
uv run pytest -p no:cacheprovider engine/tests tests/golden
pnpm contracts:check
git diff --check
```

Expected: every command exits 0; contract generation reports no stale artifact; no working diff whitespace error exists.

- [ ] **Step 5: Review scope and dependency diff**

Run:

```powershell
git diff -- Cargo.toml Cargo.lock pnpm-lock.yaml uv.lock
git diff --stat
git status --short
```

Expected: no dependency or lockfile change; only T-0111 executor, tests, and required documentation are modified.

- [ ] **Step 6: Commit documentation and evidence**

```powershell
git add docs/IMPLEMENTATION_PLAN.md data-contracts/IPC_CONTRACTS.md artifacts/THREAT_MODEL.md README.md docs/CONTEXT_PACK.md
git commit -m "docs(job): record T-0111 executor delivery"
```

- [ ] **Step 7: Final branch review**

Run:

```powershell
git log --oneline --decorate main..HEAD
git diff --check main...HEAD
git status -sb
```

Expected: focused T-0111 history contains the design, implementation plan, repository-worktree hygiene, reaper clarification, all eight implementation tasks, and any review-fix commits required by task gates; the worktree is clean and the branch contains no unrelated file changes. Verify scope from the complete `main...HEAD` diff rather than a brittle fixed commit count.

---

## Plan self-review result

- Spec coverage: every architecture, lifecycle, cancellation, preflight, panic, shutdown, recovery, security, testing, documentation, and acceptance requirement maps to Tasks 1-8.
- Scope: one Rust executor subsystem; Python dispatch, Tauri IPC/events, UI, retries, platform resource discovery, and cross-process coordination remain excluded.
- Type consistency: `ResourceBudget`, `ResourceEstimate`, `ExecutionContext`, handler outcomes, error classifications, queue API, and shutdown API are defined before their consumers.
- TDD: every behavioral task begins with a deterministic failing test and ends with focused verification plus a reviewable commit.
- Placeholder scan: no unresolved implementation placeholder remains.
