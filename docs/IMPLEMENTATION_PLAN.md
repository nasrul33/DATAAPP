# Codex-Ready Implementation Plan

## Delivery strategy
Setiap work package harus berupa task kecil, reviewable, memiliki input/output jelas, test, dan artifact update. Jangan meminta Codex membangun satu fase penuh dalam satu prompt.

## Phase 0 — Repository and quality foundation
### EPIC-000 Foundation
Objective: monorepo dapat dibangun deterministik pada Windows dan CI.

| Task | Scope | Main paths | Acceptance |
|---|---|---|---|
| T-0001 | Initialize pnpm/uv/cargo workspace | root, apps, crates, engine, packages | install/build commands pass |
| T-0002 | Shared lint/type/test configs | packages/config | strict TS, ruff, mypy, clippy active |
| T-0003 | CI quality gates | `.github/workflows` | all standard commands run |
| T-0004 | Contract generation skeleton | packages/contracts, scripts | one sample schema generates 3 languages |
| T-0005 | Desktop shell | apps/desktop | app opens with dashboard shell |
| T-0006 | Python sidecar handshake | crates/engine-host, engine | protocol/version/health verified |
| T-0007 | Logging/correlation IDs | all runtime layers | trace visible end-to-end |

Exit gate: clean clone builds, tests, launches, and verifies engine handshake.

## Phase 1 — Project, metadata, and job runtime
### EPIC-100 Project Core
- Transactional project create/open/validate.
- SQLite migrations and schema.
- Safe project directory structure.
- Append-only audit events.
- Recovery marker for interrupted write.

| Task | Scope | Main paths | Acceptance |
|---|---|---|---|
| T-0100 | Transactional project storage foundation | `crates/app-core`, `crates/filesystem`, `migrations/metadata-sqlite`, `packages/contracts` | create/open/validate survives interruption, rejects unsafe/inconsistent state, and records initial append-only audit event |
| T-0101 | Desktop project lifecycle integration | `apps/desktop`, `apps/desktop/src-tauri` | typed Tauri commands and complete create/open loading, error, permission, recovery, and empty states |

### EPIC-110 Job Runtime
- Job entity and state machine.
- Background execution, progress events, cancellation.
- Resource preflight and error envelope.
- UI job center.

| Task | Scope | Main paths | Acceptance |
|---|---|---|---|
| T-0110 | Persistent job state foundation | `crates/app-core`, `crates/filesystem`, `migrations/metadata-sqlite`, `packages/contracts` | schema-1 projects upgrade explicitly; job state/history survives restart; illegal/concurrent transitions fail atomically; interrupted active jobs remain traceable |
| T-0111 | Background job executor core | `crates/app-core` | bounded mock job runs asynchronously; progress/cancellation/failure remain persistent and audit-backed; resource preflight, panic containment, shutdown, and restart recovery are deterministic |
| T-0112 | Typed desktop job IPC foundation | `apps/desktop`, `apps/desktop/src-tauri`, `crates/app-core`, `packages/contracts` | active-project job get/list/cancel commands are schema-first and project-scoped; durable mutations emit revision-linked best-effort events; invalid, stale, schema-1, and corrupted states return safe typed errors |
| T-0113 | Job Center desktop UI | `apps/desktop` | active schema-2 projects render bounded durable job pages, revision-aware live updates, explicit lifecycle states, and confirmed cooperative cancellation without adding execution or migration behavior |
| T-0114 | Explicit active-project schema upgrade | `apps/desktop`, `apps/desktop/src-tauri`, `crates/app-core` | active schema-1 sessions upgrade only after exact project-name confirmation; lifecycle mutations serialize; schema-2 session state is published only after the project-scoped `JobStore` opens; rollback-safe failures preserve schema 1 and retry safely only after clean-source proof, while retained recovery artifacts and recovery-required corruption remain non-destructive with no retry |

T-0110 membangun persistence foundation: migration eksplisit schema 1 ke 2, snapshot `job` plus riwayat `job_event` append-only, optimistic revision/CAS, cancellation kooperatif idempotent, dan recovery `FAILED/INTERRUPTED`.

T-0111 menambahkan executor Rust project-scoped dengan worker/queue bounded, admission exactly-once dalam satu proses, progress dan cancellation melalui `JobStore`, resource preflight berbasis budget konfigurasi, panic containment, serta bounded shutdown dengan private reaper. T-0111 tidak menambahkan Python engine dispatch, command/event Tauri, platform resource probe, retry orchestration, atau UI Job Center; capability tersebut tetap dimiliki task lanjutan EPIC-110.

T-0112 mengekspos query dan cooperative cancellation melalui command Tauri bertipe, keyset page bounded, client TypeScript yang memvalidasi ulang payload, serta event lifecycle best-effort yang selalu mereferensikan snapshot durable dan revision persisten. Persistence tetap source of truth; event yang terlewat dipulihkan melalui `job_get`/`job_list`. Project schema 1 tetap read-only dan menghasilkan `PROJECT_UPGRADE_REQUIRED` untuk command job tanpa migrasi otomatis.

T-0113 menambahkan Job Center pada dashboard proyek aktif. UI memuat page keyset secara bounded, mempertahankan maksimum 100 snapshot, menggabungkan event hanya ketika revision lebih baru, dan selalu menyediakan refresh durable. Pembatalan memerlukan konfirmasi dan tetap cooperative; schema upgrade, enqueue operation, Python dispatch, dan retry runner tetap di luar scope.

T-0114 menggantikan residual schema-upgrade UI dari T-0112/T-0113 dengan command IPC `project_upgrade` khusus sesi aktif tanpa path dari UI. Nama proyek harus cocok persis, termasuk case, whitespace, dan Unicode code points. Create/open/close/upgrade memakai serialisasi lifecycle bersama; descriptor schema 2 hanya dipublikasikan setelah `JobStore` aktif. Kegagalan rollback-safe mempertahankan sesi schema 1 dan dapat dicoba ulang hanya setelah revalidasi source schema 1 yang bersih; lock/backup/marker yang tertinggal atau bukti rollback yang tidak terverifikasi menghasilkan `PROJECT_CORRUPTED` dan mempertahankan bukti recovery secara non-destruktif tanpa aksi retry. Finalisasi memindahkan lock sebelum marker sehingga commit sukses tidak dapat meninggalkan layout normal recovery-blocked. T-0114 tidak menambah migration, dependency, canonical contract schema, atau Tauri capability permission.

Exit gate: mock long job can run, cancel, fail, recover, and remain traceable after restart.

## Phase 2 — Data ingestion and profiling
### EPIC-200 Source Inspection
- CSV encoding/delimiter preview.
- Excel workbook/sheet inspection.
- Parquet metadata inspection.
- Import options validated before execution.

### EPIC-210 Dataset Versioning
- Immutable source dataset version.
- Fingerprint and schema manifest.
- Bounded preview and virtualized grid.

### EPIC-220 Profiling
- count/null/distinct/min/max/mean/median/quantiles.
- categorical frequencies.
- duplicate candidate and candidate key hints.
- profile report UI with warnings.

Accuracy gate: results cross-checked against fixed golden datasets and DuckDB reference queries.

## Phase 3 — Transformation engine
### EPIC-300 Core transforms
- select/rename/cast/filter/sort.
- fill/drop null, trim/case normalization.
- derive column using safe expression AST.

### EPIC-310 Relational transforms
- join with cardinality diagnostics.
- union with schema compatibility.
- group/aggregate.
- pivot/unpivot.

Required controls:
- preview impact;
- row count delta;
- schema delta;
- unmatched join report;
- output lineage.

## Phase 4 — Workflow DAG
### EPIC-400 Workflow model
- node definitions and typed ports.
- graph validation and cycle rejection.
- save/load versioned workflow.

### EPIC-410 Execution planner
- topological plan.
- node cache key from input fingerprints + canonical parameters + engine version.
- downstream invalidation.
- partial rerun and visual status.

Exit gate: import → clean → join → profile workflow survives restart and reruns deterministically.

## Phase 5 — Audit analytics
### EPIC-500 Exact and near duplicates
- exact duplicate rows/keys.
- normalized identity/rekening duplicate.
- configurable fuzzy matching with explainable similarity.

### EPIC-510 Statistical outliers
- IQR.
- Z-score and modified Z-score.
- group-aware populations.
- null/zero variance handling.

### EPIC-520 Benford and rule engine
- first/second digit tests with applicability warning.
- visual rule builder using safe AST.
- row-level reason codes.

### EPIC-530 Risk scoring
- weighted deterministic score.
- threshold bands.
- factor contribution explanation.
- no automatic fraud conclusion.

## Phase 6 — Visualization, findings, export
### EPIC-600 Visualization
- histogram, box, scatter, line, bar, heatmap.
- cross-filter and row locator selection.
- chart specification artifact.

### EPIC-610 Findings
- review lifecycle.
- condition/criteria/cause/effect/recommendation.
- evidence reference and snapshot hash.
- false-positive disposition.

### EPIC-620 Export
- CSV/XLSX/Parquet.
- chart PNG/SVG.
- JSON reproducibility manifest.
- exported-row and masking summary.

## Phase 7 — Hardening and release
- Crash and power-loss simulation.
- Very large file stress tests.
- Installer and sidecar packaging.
- Migration rehearsal.
- Threat model and dependency review.
- Accessibility/UAT.
- Signed release and checksums.

## MVP release acceptance
- Import/profile/transform/join/detect/visualize/export works end-to-end.
- Workflow deterministic and rerunnable.
- All derived data has lineage.
- All long tasks cancellable.
- Golden accuracy suite passes.
- Project reopening preserves state.
- No critical/high unresolved security or data-loss issue.
