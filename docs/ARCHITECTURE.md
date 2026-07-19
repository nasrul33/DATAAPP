# Architecture Blueprint

## 1. Monorepo target
```text
teratai-analytics/
├── AGENTS.md
├── apps/
│   └── desktop/                 # React/Vite UI + Tauri app
├── crates/
│   ├── app-core/                # commands, orchestration, policies
│   ├── filesystem/              # safe local file access
│   ├── engine-host/             # Python sidecar lifecycle + IPC
│   └── secure-store/            # secrets/project keys
├── engine/
│   ├── teratai_engine/
│   │   ├── importers/
│   │   ├── profiling/
│   │   ├── transforms/
│   │   ├── detectors/
│   │   ├── statistics/
│   │   ├── workflow/
│   │   ├── exports/
│   │   └── common/
│   └── tests/
├── packages/
│   ├── contracts/               # JSON Schema + generated TS/Python/Rust types
│   ├── ui/                      # UI primitives and domain components
│   ├── workflow/                # DAG model/editor contracts
│   └── config/                  # lint/ts/tailwind shared config
├── migrations/
│   └── metadata-sqlite/
├── tests/
│   ├── e2e/
│   ├── fixtures/
│   └── golden/
├── artifacts/
├── docs/
└── scripts/
```

## 2. Runtime architecture
```text
React UI
  │ typed invoke/events
  ▼
Tauri/Rust application core
  ├── validates command and authorization policy
  ├── manages project/files/jobs
  ├── persists metadata/audit events to SQLite
  └── starts and supervises Python engine sidecar
          │ versioned JSON-RPC/message protocol
          ▼
Python analytics engine
  ├── Polars lazy frames
  ├── DuckDB analytical store/query
  ├── PyArrow interchange
  └── deterministic algorithm execution
```

## 3. Storage model
### Project directory
```text
project.teratai/
├── manifest.json
├── metadata.sqlite
├── data/
│   ├── source/       # immutable imported/copy/reference metadata
│   ├── derived/      # parquet/arrow outputs
│   └── cache/        # rebuildable
├── workflows/
├── findings/
├── exports/
├── attachments/
└── logs/
```

### Data ownership
- SQLite: project metadata, datasets, operation specs, workflow, job, finding, audit event.
- Parquet/Arrow: immutable materialized datasets and previews.
- DuckDB: query execution and cache/catalog; rebuildable from manifests.
- File source: never edited by the application.

## 4. Core entities
- Project
- DataSource
- DatasetVersion
- ColumnProfile
- OperationDefinition
- OperationRun
- WorkflowDefinition
- WorkflowNode
- WorkflowEdge
- Job
- DetectorResult
- RiskAssessment
- Finding
- EvidenceReference
- ExportArtifact
- AuditEvent

## 5. Integrity constraints
- DatasetVersion immutable after status `READY`.
- Every derived DatasetVersion references exactly one successful OperationRun.
- OperationRun stores canonical parameters and engine version.
- Workflow graph must be acyclic for MVP.
- Finding must reference at least one DetectorResult or EvidenceReference.
- AuditEvent append-only; correction is a compensating event.
- Cache may be deleted; source/derived manifests may not be silently deleted.

## 6. IPC strategy
- Tauri commands for project/file/job control.
- Versioned engine protocol for analytics operations.
- Long-running operations return `job_id` immediately.
- Progress and lifecycle delivered through typed events.
- Cancellation is cooperative and idempotent.
- Large tabular payloads travel via file/Arrow reference, never giant JSON arrays.

## 7. Error taxonomy
- `VALIDATION_ERROR`
- `UNSUPPORTED_FORMAT`
- `SCHEMA_INFERENCE_ERROR`
- `DATA_INTEGRITY_ERROR`
- `RESOURCE_LIMIT`
- `ENGINE_UNAVAILABLE`
- `OPERATION_FAILED`
- `CANCELLED`
- `PERMISSION_DENIED`
- `PROJECT_CORRUPTED`

Every error includes code, user-safe message, technical detail, retriable flag, correlation ID, and optional remediation.

## 8. Security
- Offline-first; no telemetry containing user data by default.
- Explicit file picker grants only required paths.
- Project encryption becomes post-MVP unless required earlier.
- Sensitive values masked in preview/export configuration.
- Sidecar launched with controlled arguments and project-scoped access.
- Formula/rule language uses safe AST, not `eval`.
- Dependency lockfiles and SBOM generated per release.

## 9. Performance budgets
- UI interaction target <100 ms for local controls.
- Preview first page ≤2 s after dataset ready.
- Virtualized table; default preview 1,000 rows.
- Batch/chunk operations; no unbounded list collection.
- Lazy query plans where possible.
- Job memory limits and disk-space preflight checks.

## 10. Deployment
- Signed installers for Windows first; Linux second; macOS later.
- Python engine bundled as versioned sidecar.
- Reproducible release build and checksum manifest.
- Database migration runs transactionally with backup and rollback marker.
