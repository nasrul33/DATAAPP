# T-0111 Background Job Executor Core Design

## Status

Approved by the user on 2026-07-21. This document defines the next reviewable task in EPIC-110 after T-0110 persistent job state. Implementation requires a separate TDD implementation plan.

## Objective

Add a bounded, project-scoped Rust background executor that can run a deterministic mock long job, persist progress, cooperate with cancellation, contain handler failures, enforce declared resource budgets, shut down within a configured deadline, and leave every lifecycle change traceable through the T-0110 `JobStore`.

## Scope

T-0111 includes:

- an in-process `JobExecutor` owned by `crates/app-core`;
- a bounded submission queue and a configured, bounded worker count;
- exactly-once in-process admission for each `job_id`;
- a `JobHandler` boundary for deterministic, non-Python work;
- a mock long-running handler used by tests and the EPIC-110 exit-gate proof;
- cooperative cancellation checkpoints backed by the persistent job snapshot;
- progress persistence through the existing optimistic revision/CAS API;
- declared resource estimation, reservation, and preflight against explicit executor budgets;
- handler error and panic containment;
- bounded graceful shutdown and T-0110 restart recovery compatibility;
- tests and updates to architecture artifacts, IPC documentation, the implementation plan, threat model, README, and context pack.

T-0111 excludes:

- Python engine dispatch or analytics execution;
- arbitrary user Python or executable payloads;
- Tauri job commands and frontend events;
- Job Center UI;
- automatic retry;
- cross-process or multi-instance executor coordination;
- persistence of operation-specific payloads;
- physical memory or free-disk discovery. The application must supply explicit budgets; later engine integration may derive those budgets from platform probes.

## Alternatives considered

### Selected: in-process Rust executor

Rust worker threads consume a bounded queue and invoke typed handlers. This approach reuses `JobStore`, avoids a second IPC protocol, supports deterministic unit tests, and isolates EPIC-110 execution semantics from Python analytics behavior.

### Rejected for T-0111: dedicated executor process

A separate process gives stronger isolation but introduces supervision, another protocol, and crash coordination before the engine command contract exists.

### Rejected for T-0111: direct Python sidecar dispatch

Dispatching immediately to the sidecar would combine executor semantics, engine protocol expansion, operation payload design, and Python cancellation in one change. Those concerns remain a later task after the core executor contract is proven.

## Architecture

`JobStore` remains the sole authority for durable job state, revision checks, job events, and audit events. `JobExecutor` owns only volatile admission, scheduling, worker lifecycle, and resource reservations. It never edits SQLite directly.

The executor is project-scoped. Construction requires an already-open schema-2 `JobStore`, a handler registry, an explicit `JobExecutorConfig`, and an injectable clock. The executor does not open arbitrary project paths and does not accept source rows, dataset values, raw scripts, or unbounded JSON payloads.

The implementation uses standard-library synchronization and threading only. No runtime dependency is added.

## Components and responsibilities

### `JobExecutor`

- validates configuration before starting workers;
- accepts only a bounded `job_id` submission;
- prevents the same `job_id` from being queued or active twice in one executor;
- places accepted IDs into a bounded FIFO queue without waiting indefinitely;
- owns worker shutdown coordination and joins every worker before returning from successful shutdown;
- releases admission and resource reservations on every success, error, cancellation, and panic path.

### `JobHandler`

Each registered job kind maps to one typed handler. A handler exposes:

- its stable job kind;
- a deterministic `ResourceEstimate` before execution;
- a `run` method receiving only an `ExecutionContext`.

Handlers cannot mutate `JobStore` directly. They report progress and observe cancellation through the context so lifecycle policy stays centralized.

### `ExecutionContext`

The context exposes:

- `checkpoint(progress)`, which persists a validated progress update using the latest executor-owned revision;
- `cancellation_requested()`, which reads the current trusted snapshot and returns true only for `CANCELLING`;
- the immutable `job_id`, project ID, and correlation ID;
- the injected clock required for deterministic timestamps.

Every handler loop must call a checkpoint at bounded work intervals. The mock handler uses an injected checkpoint gate rather than wall-clock sleeps, making cancellation and shutdown tests deterministic.

### `ResourceLedger`

Each handler declares non-negative `memory_bytes`, `disk_bytes`, and one ordered `DurationClass` (`SHORT`, `MEDIUM`, or `LONG`) before state changes. The ledger atomically reserves byte estimates against explicit executor-wide memory and disk budgets and rejects a duration class above the configured maximum. A job whose single estimate, aggregate reservation, or duration class exceeds a budget fails preflight without invoking the handler. Reservations use checked arithmetic and are always released through an RAII guard.

T-0111 does not claim that configured budgets equal current physical availability. The desktop/engine integration task will own platform probes and budget selection.

### `WorkerReaper`

Construction preallocates one `Arc<WorkerSlot>` per worker and registers clones of all slots with one executor-private maintenance reaper before any worker starts. Each spawned worker installs its `JoinHandle` in its shared slot; handles are never sent through the command channel. The reaper normally receives the payload-free `Stop` command and is joined after graceful shutdown has taken/joined every worker handle. If `Drop` follows a timed-out explicit shutdown, dropping the command sender disconnects and wakes the reaper; it takes and joins every remaining slot handle asynchronously and then exits. `Full` or `Disconnected` command-channel outcomes cannot own or drop a handle, and the caller-facing destructor performs no unbounded wait.

## Configuration

`JobExecutorConfig` has no implicit defaults or hidden thresholds. Its native owner must supply:

- `worker_count`: positive integer no greater than `std::thread::available_parallelism()`;
- `queue_capacity`: positive integer whose bookkeeping allocation succeeds through checked construction;
- `memory_budget_bytes`: positive integer;
- `disk_budget_bytes`: positive integer;
- `max_duration_class`: `SHORT`, `MEDIUM`, or `LONG`;
- `shutdown_timeout`: positive duration.

Invalid configuration fails before any worker starts. All operational limits remain explicit policy input and are recorded in tests and calling configuration rather than hidden inside executor behavior.

## Submission and execution flow

1. The caller persists a job through `JobStore::enqueue`.
2. The caller submits only its `job_id` to `JobExecutor::submit`.
3. Submission validates the identifier, verifies the snapshot is `QUEUED`, verifies a handler exists for its kind, and claims the ID in the volatile admission set.
4. A non-blocking bounded queue send succeeds or returns a typed `QueueFull` error. A failed send releases the admission claim and does not mutate persistent state.
5. A worker re-reads the job. If it is no longer `QUEUED`, execution is skipped and the claim is released.
6. The handler estimate is validated and reserved. Rejected preflight persists `FAILED/RESOURCE_LIMIT` with a safe Indonesian message and never calls the handler.
7. The worker transitions the job to `RUNNING` through CAS.
8. The handler runs inside a panic boundary. Progress checkpoints update persistence and refresh the worker-owned revision.
9. If the snapshot becomes `CANCELLING`, the next checkpoint stops handler work and completes the persistent transition to `CANCELLED`.
10. Normal return transitions to `SUCCEEDED`. A typed handler failure transitions to `FAILED/OPERATION_FAILED`. A panic transitions to `FAILED/OPERATION_FAILED` using a fixed safe message; panic text is never persisted or returned.
11. The worker releases the resource reservation and admission claim on every terminal path.

## Cancellation semantics

Cancellation remains cooperative and idempotent as defined by T-0110. T-0111 does not add a second cancellation flag that could disagree with SQLite.

- A queued job changed to `CANCELLING` before worker start is never passed to its handler. The worker completes it as `CANCELLED`.
- A running handler detects `CANCELLING` at its next checkpoint and stops before doing more work.
- Repeated cancellation requests do not create duplicate events.
- A terminal job ignores executor cancellation observation because T-0110 terminal states are immutable.

The handler contract requires bounded checkpoint intervals. T-0111 cannot forcibly terminate a non-cooperative handler thread; such a handler is a programming defect and remains visible as a shutdown timeout.

## Concurrency and revision handling

The volatile admission set prevents duplicate execution within one executor. `JobStore` revision/CAS remains the durable concurrency boundary against callers and independent store handles.

The worker owns the latest revision only between store calls. Before each transition or progress update it re-reads the trusted snapshot when cancellation or an external mutation may have occurred. A stale revision is reconciled once by re-reading state:

- `CANCELLING` follows the cancellation path;
- a terminal state stops execution without another write;
- any other incompatible state returns `PersistenceConflict`, releases resources, and does not invent a compensating transition.

T-0111 guarantees no duplicate execution only inside one process and one project-scoped executor. Cross-process coordination is explicitly deferred.

## Shutdown and restart recovery

Shutdown has three phases:

1. atomically stop accepting submissions;
2. close the queue and allow queued/running handlers to reach their next checkpoint;
3. join workers until the configured deadline.

Queued jobs not started before shutdown remain `QUEUED`. Running or cancelling jobs that do not finish remain in their current persistent state. The executor does not falsely mark them successful or delete their history. On the next application start, the existing T-0110 `recover_interrupted` operation changes `RUNNING` and `CANCELLING` jobs to `FAILED/INTERRUPTED` exactly once.

If all workers and the idle reaper join within the deadline, shutdown succeeds. Otherwise it returns `ShutdownTimeout` and retains outstanding worker handles in their shared slots for a later shutdown attempt. If the executor is then dropped, sender disconnection wakes the already-running reaper to take/join those slot handles without an unbounded caller wait. The executor never kills a worker or claims bounded shutdown success while work remains; application termination remains an outer-process decision.

## Typed errors

Executor errors use stable classifications and user-safe messages:

- `InvalidConfiguration`;
- `InvalidJobId`;
- `JobNotFound`;
- `InvalidState`;
- `HandlerNotFound`;
- `AlreadySubmitted`;
- `QueueFull`;
- `PreflightRejected`;
- `PersistenceConflict`;
- `PersistenceFailed`;
- `ShuttingDown`;
- `ShutdownTimeout`.

Handler failures use stable handler codes but are normalized to the persistent job error envelope. Raw panic payloads, database errors, absolute paths, source values, and secrets never enter errors, logs, progress, or audit events.

## Security and integrity

- The executor accepts identifiers, not paths or executable payloads.
- Only handlers registered by native code can execute.
- Queue, worker count, progress messages, and resource arithmetic are bounded.
- `JobStore` remains responsible for validating persisted rows before they influence execution.
- Every material lifecycle mutation remains atomic with append-only job and audit events.
- Source files are never modified in place.
- No telemetry or user data leaves the process.

## Testing strategy

Tests use an injectable clock, deterministic checkpoint gates, fake resource estimates, and a configurable handler registry. They must not depend on scheduler sleeps for correctness.

Required tests:

- successful mock job reaches `RUNNING`, persists monotonic progress, then reaches `SUCCEEDED`;
- queued cancellation prevents handler invocation and reaches `CANCELLED`;
- running cancellation is observed at a deterministic checkpoint and reaches `CANCELLED`;
- duplicate submission never invokes the handler twice;
- a full queue returns immediately and leaves persistence unchanged;
- unknown handler kind is rejected without lifecycle mutation;
- resource estimate overflow and aggregate budget exhaustion fail safely;
- preflight rejection reaches `FAILED/RESOURCE_LIMIT` without handler invocation;
- handler typed failure reaches `FAILED/OPERATION_FAILED`;
- handler panic is contained, uses a fixed safe error, and does not stop another worker;
- concurrent cancellation and progress reconcile through trusted state and CAS;
- graceful shutdown joins cooperative workers within the deadline;
- shutdown timeout is explicit for a deliberately non-cooperative test handler;
- queued jobs remain queued across shutdown;
- active jobs left by interrupted execution are recovered exactly once through T-0110;
- a deterministic multithread stress test proves one invocation per admitted `job_id`;
- event and audit counts/hashes remain consistent for every terminal path.

The final milestone gate remains the full repository command matrix from `AGENTS.md`.

## Documentation and traceability

Implementation must:

- add T-0111 to `docs/IMPLEMENTATION_PLAN.md` with explicit acceptance criteria;
- update `data-contracts/IPC_CONTRACTS.md` without claiming Tauri or engine dispatch support;
- document executor trust boundaries and panic/resource behavior in `artifacts/THREAT_MODEL.md`;
- add verified setup/test commands to `README.md` if new commands are introduced;
- append T-0111 decisions, deviations, evidence, and delivery status to `docs/CONTEXT_PACK.md`.

## Acceptance criteria

T-0111 is complete only when:

- a persisted mock long job runs asynchronously and remains traceable through success;
- queued and running cancellation are cooperative, deterministic, and audit-backed;
- progress is bounded, monotonic under T-0110 rules, and survives store reopen;
- resource preflight rejects over-budget work before handler execution;
- duplicate submission cannot execute a job twice in one executor;
- handler errors and panics become safe terminal failures without stopping unrelated workers;
- shutdown is bounded and restart recovery preserves interrupted work;
- no Python dispatch, Tauri job command/event, UI Job Center, retry runner, or arbitrary execution is added;
- no runtime dependency is added;
- all required quality gates pass.
