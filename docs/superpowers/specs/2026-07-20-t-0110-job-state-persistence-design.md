# T-0110 Job State Persistence Design

## Status

Approved for specification by the user on 2026-07-20. This document defines the first reviewable task in EPIC-110. Implementation requires a separate TDD implementation plan and must not expand into background execution or UI work.

## Objective

Add a project-scoped, persistent Job entity and deterministic state machine backed by metadata SQLite schema 2. Jobs must remain traceable after restart, reject illegal or concurrent transitions, and preserve the existing read-only semantics of project open and validation.

## Scope

T-0110 includes:

- metadata migration `1 -> 2` with recovery and rollback documentation;
- explicit project upgrade support without implicit mutation during open or validation;
- `job` current-state persistence;
- append-only `job_event` history;
- transactional audit events for job lifecycle changes;
- a Rust `JobStore` in `crates/app-core`;
- bounded get/list queries;
- deterministic interruption recovery;
- canonical cross-language job contracts required by the core API;
- tests and updates to the data dictionary, IPC documentation, implementation plan, threat model, and context pack.

T-0110 excludes:

- worker threads or asynchronous executors;
- Python sidecar job commands;
- Tauri job commands and frontend events;
- Job Center UI;
- retry execution and resource preflight;
- operation-specific payloads or arbitrary JSON metadata;
- automatic migration during project open or validation.

## Architecture decision

Job persistence remains in `crates/app-core` because that crate already owns project orchestration and metadata transactions. A new crate would create an abstraction without an executor, while implementing the transition policy primarily in SQLite triggers would hide application behavior and make typed errors harder to test.

SQLite enforces durable shape and append-only history. Rust owns transition policy, optimistic concurrency, validation, typed failures, and transaction orchestration. Every state change uses one `BEGIN IMMEDIATE` transaction that updates the current snapshot, appends a job event, and appends an audit event before commit.

No new runtime dependency is required.

## Metadata schema 2

### `job`

The `job` table stores the latest trusted snapshot:

| Column | Type | Rules |
|---|---|---|
| `job_id` | TEXT | primary key, lowercase UUID v7 |
| `project_id` | TEXT | foreign key to the singleton project identity |
| `kind` | TEXT | stable non-empty identifier, maximum 120 characters |
| `status` | TEXT | `QUEUED`, `RUNNING`, `SUCCEEDED`, `FAILED`, `CANCELLING`, or `CANCELLED` |
| `correlation_id` | TEXT | lowercase UUID v7 for the initiating request |
| `revision` | INTEGER | starts at 1 and increases exactly once per material state/progress mutation |
| `created_at` | TEXT | UTC RFC 3339 timestamp |
| `started_at` | TEXT | nullable; set only when entering `RUNNING` |
| `finished_at` | TEXT | nullable; required for terminal states only |
| `updated_at` | TEXT | UTC RFC 3339 timestamp for the latest committed mutation |
| `progress_current` | INTEGER | non-negative, default 0 |
| `progress_total` | INTEGER | nullable positive total; current may not exceed total |
| `progress_unit` | TEXT | nullable bounded unit label |
| `progress_phase` | TEXT | nullable bounded stable phase identifier |
| `progress_message` | TEXT | nullable bounded user-safe message |
| `error_code` | TEXT | nullable; required for `FAILED` |
| `error_message` | TEXT | nullable bounded user-safe message; required for `FAILED` |
| `error_retriable` | INTEGER | nullable boolean represented as 0/1; required for `FAILED` |

The table does not store source rows, dataset values, absolute paths, raw exceptions, or arbitrary JSON. Indexes support status/updated ordering and correlation lookup.

### `job_event`

The `job_event` table is the immutable lifecycle timeline:

| Column group | Purpose |
|---|---|
| `sequence`, `event_id`, `job_id` | deterministic ordering and identity |
| `event_type`, `from_status`, `to_status` | explicit transition or progress event |
| progress snapshot columns | the bounded progress state relevant to the event |
| error columns | the safe failure state relevant to the event |
| `occurred_at`, `correlation_id` | UTC ordering and trace correlation |

Update and delete triggers abort all mutations. Indexes support `(job_id, sequence)` and `(correlation_id, sequence)` reads. Corrections are new events, never edits.

### Audit integration

Each material job mutation appends one `audit_event` in the same transaction. Stable actions are:

- `job.queued`
- `job.started`
- `job.progressed`
- `job.cancellation_requested`
- `job.cancelled`
- `job.succeeded`
- `job.failed`
- `job.interrupted`

The audit target type is `job`, target ID is `job_id`, and the correlation ID matches the job command. Before/after hashes are deterministic SHA-256 hashes of the canonical persisted job snapshot.

## Project migration and compatibility

### Read-only compatibility

`ProjectService::open` and `ProjectService::validate` remain read-only. They accept supported metadata schema versions 1 and 2, validate the manifest against the actual SQLite `user_version`, and return the detected version in `ProjectDescriptor`. They reject versions newer than the binary supports.

`JobStore` requires metadata schema 2. Opening it against schema 1 returns `IncompatibleSchema` without changing the project.

### Explicit upgrade

`ProjectService::upgrade(path, correlation_id)` is the only schema 1 to schema 2 mutation entry point in T-0110. It performs these ordered stages:

1. Validate the schema 1 project read-only.
2. Create a durable metadata backup in the project recovery area.
3. Write a bounded upgrade recovery marker containing the project ID, correlation ID, source version, target version, original manifest hash, backup identity, and current stage.
4. Build the new manifest bytes with `metadata_schema_version = 2` and calculate their SHA-256 before database mutation.
5. Apply `0002_job_runtime.sql` inside one `BEGIN IMMEDIATE` transaction, record `schema_migrations` version 2, append `project.metadata_migrated` with the old and new manifest hashes, and set `PRAGMA user_version = 2`.
6. Atomically replace `manifest.json` with the pre-hashed schema 2 manifest.
7. Validate layout, SQLite integrity, migration history, identity, and the authorized manifest hash chain.
8. Remove the recovery marker and migration backup only after validation succeeds.

If any stage fails, the recovery workflow restores both metadata and manifest to their validated schema 1 state from the marker and backup. If restoration cannot be proven complete, the marker remains and normal open fails with `RecoveryRequired`; no automatic deletion occurs.

Manifest validation no longer assumes that `project.created` is the only authorized manifest hash. It verifies an unbroken audit chain beginning at `project.created` and followed only by recognized `project.metadata_migrated` events whose `before_hash` equals the previous authorized hash and whose `after_hash` equals the next manifest hash.

Successful migration has no in-place downgrade. The rollback note documents application rollback behavior and recovery of an interrupted migration; it never instructs removal of job or audit data from a user project.

## State machine

Allowed transitions are:

```text
QUEUED -> RUNNING
QUEUED -> CANCELLING
QUEUED -> FAILED
RUNNING -> SUCCEEDED
RUNNING -> FAILED
RUNNING -> CANCELLING
CANCELLING -> CANCELLED
CANCELLING -> FAILED
```

`SUCCEEDED`, `FAILED`, and `CANCELLED` are terminal. Terminal state mutation is always rejected.

Progress updates are allowed only in `RUNNING` or `CANCELLING`. Current progress cannot be negative or exceed a present total. A total must be positive. Progress must not regress within the same phase. A phase change may reset current to zero while retaining a valid total.

Cancellation is cooperative and idempotent. Requesting cancellation for `QUEUED` or `RUNNING` enters `CANCELLING`. Repeating the request while already `CANCELLING` returns the unchanged snapshot without increasing revision or appending duplicate job/audit events. Completing cancellation is valid only from `CANCELLING`.

Every mutating command supplies `expected_revision`. The update predicate includes that revision; a stale caller receives `RevisionConflict` and no event is written.

## Rust API

`JobStore` is opened for one validated project and exposes:

- `enqueue`
- `get`
- `list`
- `start`
- `update_progress`
- `request_cancellation`
- `complete_cancellation`
- `succeed`
- `fail`
- `recover_interrupted`

`list` uses deterministic `(updated_at DESC, job_id DESC)` ordering, a maximum page size of 100, and a typed Rust cursor containing that ordering tuple. Empty results are valid. Cursor serialization across IPC is deferred to the Tauri integration task.

Each mutation follows this transaction flow:

```text
validate request
-> open project metadata
-> BEGIN IMMEDIATE
-> read status and revision
-> validate transition
-> update job snapshot with expected revision
-> append job_event
-> append audit_event
-> COMMIT
```

## Restart recovery

`recover_interrupted(correlation_id, recovered_at)` finds non-terminal `RUNNING` and `CANCELLING` jobs from a previous process and transitions each to `FAILED` with:

- `error_code = INTERRUPTED`;
- a bounded Indonesian user-safe message;
- `error_retriable = 1`;
- a `job.interrupted` event and audit action.

It does not claim cancellation succeeded, delete outputs, or retry work. `QUEUED` jobs remain queued for a future scheduler. Repeating recovery is idempotent because no recoverable rows remain after the first successful transaction.

## Error model

The Rust API returns typed failures:

- `InvalidRequest`
- `JobNotFound`
- `InvalidTransition`
- `RevisionConflict`
- `IncompatibleSchema`
- `DataIntegrity`
- `Database`
- `Timestamp`

Errors retain safe structured context needed by later adapters, but raw SQLite failures and absolute project paths are not promoted into canonical contracts or UI messages. Tauri mapping remains owned by the next integration task.

## Canonical contracts

Schema-first contracts define the stable cross-language shapes required by the persistence boundary:

- job descriptor/current snapshot;
- enqueue request;
- transition request with expected revision;
- progress update request;
- failure request.

Contracts remain flat, additive, and generator-revision-1 compatible. They contain no nested arbitrary objects, object arrays, or maps. Status values are validated at runtime because the current generator subset does not emit enums. Bounded list pagination remains a Rust API in T-0110; its cross-language page envelope belongs to the later Tauri integration task, which must either use a generator-supported representation or explicitly extend the generator with tests.

## Test strategy

Implementation follows strict red-green-refactor. Tests must first fail for the missing behavior and then cover:

- schema 1 project validation without mutation;
- successful explicit migration from 1 to 2 while preserving project identity and all prior audit events;
- migration failure restoration without a partially upgraded database;
- recovery marker retention when restoration cannot be proven;
- rejection of unsupported newer schemas;
- enqueue persistence and deterministic bounded listing;
- every allowed transition;
- every illegal transition and terminal-state mutation;
- optimistic revision conflict with no partial events;
- progress bounds and phase reset behavior;
- idempotent cancellation request;
- append-only job event triggers;
- atomic job/event/audit writes under an injected database failure;
- restart recovery to `FAILED/INTERRUPTED`;
- idempotent repeated recovery;
- canonical contract generation and three-language fixtures.

Full repository gates remain mandatory after targeted tests.

## Acceptance criteria

T-0110 is complete when:

- schema 2 can be created and schema 1 can be explicitly upgraded without data or audit loss;
- open and validate remain read-only for supported schema 1 and 2 projects;
- persistent job snapshots and append-only histories survive close/reopen;
- all transition, concurrency, cancellation, progress, and recovery rules are enforced transactionally;
- interrupted active jobs remain traceable as `FAILED/INTERRUPTED` after recovery;
- no new runtime dependency, background executor, Tauri command, Python job command, or UI behavior is introduced;
- contracts, migration/rollback notes, data dictionary, threat model, IPC documentation, implementation plan, tests, and context pack agree;
- all standard quality gates pass without warnings.

## Follow-up boundaries

The next EPIC-110 task may add a supervised mock background executor, resource preflight, and cooperative cancellation using this store. A later task owns typed Tauri job commands/events and the Job Center UI. Neither follow-up may bypass the state machine or write metadata tables directly.
