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
- T-0004 adds `ContractMetadata` as a generation/serialization proof only. It does not add an IPC command or event.

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
