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
- T-0100 adds project create request, manifest, and validated descriptor contracts. Native Tauri command exposure remains owned by T-0101.

## Project lifecycle contract

`ProjectCreateRequest` carries distinct UUID v7 request/project identifiers, an absolute user-approved `.teratai` target, and a validated display name. Project creation never writes directly into the final target: the native core initializes a recovery-marked sibling staging directory, applies metadata migrations transactionally, writes a bounded manifest atomically, validates integrity, then publishes with a same-volume rename.

`ProjectManifest` is the immutable compatibility and identity record. `ProjectDescriptor` is returned only after directory layout, manifest version, SQLite `integrity_check`, metadata schema version, project identity, initial audit event, and manifest fingerprint agree. Open/validate are read-only and reject recovery markers or linked control files.

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
