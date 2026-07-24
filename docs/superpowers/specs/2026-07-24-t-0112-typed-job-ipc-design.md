# T-0112 Typed Desktop Job IPC Design

## Objective

Expose the durable T-0110/T-0111 job lifecycle to the Tauri desktop boundary without adding analytics execution or Job Center UI.

## Scope

- canonical `JobGetRequest`, `JobListRequest`, `JobListResponse`, and `JobLifecycleEvent`;
- generator revision 2 support for validated, acyclic sibling schema references;
- Tauri `job_get`, `job_list`, and `job_cancel` commands;
- active-project and schema-version isolation;
- best-effort `job:lifecycle` notifications after durable commit and metadata identity refresh;
- strict TypeScript parsing for descriptors, pages, events, and native error envelopes.

## Command contracts

`job_get` returns one trusted `JobDescriptor` from the active project.

`job_list` accepts a page size from 1 through 100 and an optional complete keyset cursor pair. It returns newest-first items plus either both next-cursor fields or neither.

`job_cancel` reuses `JobTransitionRequest`, including the current positive revision. Cancellation remains cooperative and idempotent through `JobStore`; the desktop adapter never writes SQLite directly.

All commands require a lowercase UUID v7 correlation ID. Job identity lookup is restricted to the active project's pinned `JobStore`.

## Session and compatibility

The desktop project session owns an optional `Arc<JobStore>` beside the trusted descriptor. Schema-2 create/open activates the store with the Tauri event sink. Schema-1 open remains valid and read-only, but job commands return `PROJECT_UPGRADE_REQUIRED`. Open never performs migration.

Closing the project drops the active store reference and makes subsequent job commands return a safe validation error.

## Event contract

Channel: `job:lifecycle`.

Payload: `JobLifecycleEvent` containing protocol version `1.0`, a stable event name, positive sequence, mutation timestamp, and the complete trusted post-mutation `JobDescriptor`.

The sequence equals `job.revision` and `occurred_at` equals `job.updated_at`. Notifications are published only after the mutation transaction commits and pinned metadata identity refresh succeeds. Delivery failure cannot roll back or alter the durable lifecycle.

Supported names:

- `job.queued`
- `job.started`
- `job.progress`
- `job.completed`
- `job.failed`
- `job.cancellation_requested`
- `job.cancelled`

Events are an optimization, not a second state store. Consumers recover missed events with `job_get` or `job_list`.

## Error mapping

- invalid IDs, limits, cursors, and missing jobs: `VALIDATION_ERROR`;
- stale revision: retriable `OPERATION_FAILED`;
- invalid transition: non-retriable `OPERATION_FAILED`;
- schema 1: `PROJECT_UPGRADE_REQUIRED`;
- descriptor or metadata integrity failure: `PROJECT_CORRUPTED`;
- database/timestamp failure: retriable `OPERATION_FAILED`.

Messages and details are fixed, bounded, user-safe, and contain no raw database errors or absolute paths.

## Out of scope

- Job Center UI;
- Python engine dispatch;
- operation-specific enqueue commands or executable payloads;
- automatic retry;
- platform resource probes;
- project schema upgrade UI;
- durable event broker or cross-process delivery.

## Acceptance

- all four canonical contracts generate deterministically in TypeScript, Python, and Rust;
- command integration tests prove persistent get/list/cancel behavior in one active project;
- lifecycle tests prove one event per durable mutation, contiguous revision sequence, and no duplicate event for idempotent cancellation;
- malformed page/event/descriptor payloads fail the TypeScript trust boundary;
- all repository quality gates pass without new dependencies, migration, or source-data access.
