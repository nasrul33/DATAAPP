# IPC and Engine Contracts

## Contract principles
- Schema-first and versioned.
- Generate TS, Python, and Rust types from one canonical schema.
- Unknown additive fields tolerated; breaking fields require protocol major version.
- IDs use UUID v7 where available.
- Timestamps are ISO-8601 UTC; UI localizes to Asia/Jakarta.
- Decimal-sensitive values use decimal strings plus scale, not binary float where exactness matters.

## Generation workflow
- Canonical machine-readable schemas live in `packages/contracts/schemas`.
- Run `pnpm contracts:generate` after changing a schema.
- Run `pnpm contracts:check` to detect missing or stale TypeScript, Python, or Rust output.
- Generated files include a canonical schema SHA-256 and must never be edited manually.
- Generator revision 1 supports the bounded subset documented in `packages/contracts/README.md`; unsupported constructs fail generation.
- T-0004 adds `ContractMetadata` as a generation/serialization proof.
- T-0006 adds canonical engine handshake request/response and typed engine error schemas. Runtime messages use bounded newline-delimited JSON over controlled process stdio.
- T-0007 adds `RuntimeLogEvent` as the canonical structured runtime trace shared by TypeScript, Rust, and Python.
- T-0100 adds project create request, manifest, and validated descriptor contracts.
- T-0101 adds correlation/open request and desktop error contracts, then exposes the lifecycle through typed Tauri commands.
- T-0110 adds five flat persistent-job contracts: `JobDescriptor`, `JobEnqueueRequest`, `JobTransitionRequest`, `JobProgressUpdateRequest`, and `JobFailureRequest`.

## Project lifecycle contract

`ProjectCreateRequest` carries distinct UUID v7 request/project identifiers, an absolute user-approved `.teratai` target, and a validated display name. Project creation never writes directly into the final target: the native core initializes a recovery-marked sibling staging directory, applies metadata migrations transactionally, writes a bounded manifest atomically, validates integrity, then publishes with a same-volume rename.

`ProjectManifest` is the immutable compatibility and identity record. `ProjectDescriptor` is returned only after directory layout, manifest version, SQLite `integrity_check`, metadata schema version, project identity, initial audit event, and manifest fingerprint agree. Open/validate are read-only and reject recovery markers or linked control files.

The desktop adapter exposes `project_create`, `project_open`, `project_validate`, `project_current`, and `project_close`. Every request carries a UUID v7 `correlation_id`; create/open/validate return a trusted `ProjectDescriptor`, current returns the active descriptor or `null`, and close clears only the in-memory session. Native failures cross the boundary as `DesktopError` with a stable code, localized safe message, retriable flag, correlation ID, and bounded field errors. Raw database errors and absolute paths are never returned as error detail.

Project selection uses the operating-system dialog. Only `dialog:allow-open` and `dialog:allow-save` are granted to the main window; arbitrary frontend filesystem access is not enabled.

### Metadata schema 1 to 2

Open and validate are read-only for supported metadata schema 1 and 2. Upgrade is explicit through the native project service; it is never an open-time side effect. Before mutation, schema-1 `metadata.sqlite` and `manifest.json` are copied and synced as recovery proofs and a bounded marker blocks normal open. Migration 0002, its `schema_migrations` row, and the hash-linked `project.metadata_migrated` audit event commit in one `BEGIN IMMEDIATE` transaction; the schema-2 manifest is then atomically replaced and the complete project is revalidated before recovery artifacts are cleared.

There is no destructive schema-2-to-1 downgrade. Older binaries must reject schema 2. A failed upgrade restores both schema-1 control files byte-for-byte and revalidates them; if that proof fails, artifacts remain and the project stays recovery-required.

## Persistent job contracts (T-0110)

All five contracts are flat, schema-first, additive (`additionalProperties: true`), and generator-revision-1 compatible. Optional persistence values are optional properties rather than nested/nullable union types. Runtime validation remains native because generator revision 1 does not emit enums or format validators.

| Contract | Required | Optional |
|---|---|---|
| `JobDescriptor` | `job_id`, `project_id`, `kind`, `status`, `correlation_id`, `revision`, `created_at`, `updated_at`, `progress_current` | `started_at`, `finished_at`, `progress_total`, `progress_unit`, `progress_phase`, `progress_message`, `error_code`, `error_message`, `error_retriable` |
| `JobEnqueueRequest` | `job_id`, `kind`, `correlation_id` | `progress_total`, `progress_unit` |
| `JobTransitionRequest` | `job_id`, `correlation_id`, `expected_revision` | - |
| `JobProgressUpdateRequest` | `job_id`, `correlation_id`, `expected_revision`, `current`, `phase`, `message` | `total`, `unit` |
| `JobFailureRequest` | `job_id`, `correlation_id`, `expected_revision`, `error_code`, `error_message`, `error_retriable` | - |

`job_id`, `project_id`, dan `correlation_id` adalah lowercase UUID v7. `kind`, phase, dan unit adalah bounded safe identifiers; progress/error messages adalah trimmed, control-character-free safe text maksimum 500 bytes; error code adalah uppercase identifier maksimum 120 bytes. Revision harus positif. Timestamps adalah UTC RFC 3339 dan tidak boleh mundur terhadap snapshot sebelumnya. `progress_current` tidak negatif, total bila ada harus positif, dan current tidak boleh melampaui total.

### Transition and atomicity rules

Transition yang diizinkan tepatnya:

```text
QUEUED     -> RUNNING | CANCELLING | FAILED
RUNNING    -> SUCCEEDED | CANCELLING | FAILED
CANCELLING -> CANCELLED | FAILED
SUCCEEDED  -> (none)
FAILED     -> (none)
CANCELLED  -> (none)
```

Setiap enqueue dan mutasi material memperbarui/menulis snapshot `job`, menambahkan satu `job_event`, dan menambahkan satu `audit_event` berisi before/after snapshot hash di dalam satu `BEGIN IMMEDIATE` transaction. Semua mutasi selain enqueue membawa `expected_revision`; update menggunakan CAS `WHERE revision = expected_revision`, menaikkan revision tepat satu, dan stale writer gagal tanpa event parsial. History `job_event` dan `audit_event` tidak dapat di-update/delete; koreksi harus berupa event baru.

Progress hanya diizinkan ketika status `RUNNING` atau `CANCELLING`. Current tidak boleh turun dalam phase yang sama; pergantian phase boleh mereset current selama bounds tetap valid. Cancellation bersifat kooperatif: request dari `QUEUED`/`RUNNING` masuk `CANCELLING`; request ulang dengan revision saat ini pada `CANCELLING` mengembalikan snapshot yang sama tanpa menaikkan revision atau menulis event; completion hanya valid dari `CANCELLING` ke `CANCELLED`.

Recovery restart memilih hanya `RUNNING` dan `CANCELLING`, lalu dalam satu transaksi mengubah masing-masing menjadi terminal `FAILED`, `error_code = INTERRUPTED`, safe Indonesian message, `error_retriable = true`, dan event `job.interrupted`. `QUEUED` serta semua terminal state tetap utuh; recovery ulang setelah sukses tidak menulis duplicate history.

T-0110 menyediakan persistence/store API dan bounded Rust keyset listing, bukan executor, engine dispatch, retry runner, Tauri job command/event, page envelope lintas bahasa, atau UI job center.

## Engine startup handshake

The native host launches the configured Python 3.12 executable with an explicit engine module root, sends `engine.handshake`, and verifies:

- request correlation;
- protocol version `1.0`;
- engine package version;
- Python `3.12.x` runtime;
- `ready` health state and the `health` capability.

The sidecar remains alive after a successful handshake and exits normally when the host closes stdin. Startup timeout and shutdown timeout are mandatory; shutdown falls back to process termination without modifying project data.

## Runtime trace contract

Every runtime event contains an ISO-8601 UTC timestamp, `DEBUG|INFO|WARNING|ERROR` level, `desktop|native|engine` layer, component, stable event name, safe message, UUID v7 `correlation_id`, and positive sequence number. Sequence is the deterministic cross-layer ordering key; wall-clock timestamps are diagnostic metadata only.

The desktop starts a trace, the native host validates correlation and collects a bounded in-memory trace, and the engine emits structured events on stderr. Engine stdout remains reserved for bounded IPC protocol messages. Invalid or unstructured stderr is isolated as bounded diagnostics and is never promoted into the canonical trace.

Runtime messages must not contain source rows, dataset values, secrets, credentials, or absolute filesystem paths. New metadata fields require schema review before use; arbitrary context maps are intentionally unsupported.

## Command envelope
```json
{
  "protocol_version": "1.0",
  "request_id": "uuid",
  "command": "dataset.profile",
  "project_id": "uuid",
  "payload": {},
  "trace": { "correlation_id": "uuid" }
}
```

## Accepted response
```json
{
  "request_id": "uuid",
  "accepted": true,
  "job_id": "uuid"
}
```

## Event envelope
```json
{
  "protocol_version": "1.0",
  "event": "job.progress",
  "job_id": "uuid",
  "sequence": 12,
  "payload": {
    "status": "RUNNING",
    "phase": "profiling.columns",
    "current": 7,
    "total": 20
  }
}
```

## Error envelope
```json
{
  "code": "VALIDATION_ERROR",
  "message": "Parameter kolom tidak valid.",
  "detail": "column_id does not exist in dataset version",
  "retriable": false,
  "correlation_id": "uuid",
  "field_errors": []
}
```

## Mandatory commands MVP
- `project.create/open/close/validate`
- `source.inspect/import`
- `dataset.preview/schema/profile`
- `operation.validate/run/cancel`
- `workflow.validate/run/save`
- `job.get/list/cancel`
- `finding.create/update/list`
- `export.validate/run`

## Mandatory events MVP
- `job.queued`
- `job.started`
- `job.progress`
- `job.warning`
- `job.completed`
- `job.failed`
- `job.cancelled`
- `project.changed`
