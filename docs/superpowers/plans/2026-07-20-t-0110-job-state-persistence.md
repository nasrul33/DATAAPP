# T-0110 Job State Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build metadata schema 2 with explicit schema-1 upgrade, persistent project-scoped jobs, deterministic optimistic-concurrency transitions, append-only history, audit integration, and restart recovery.

**Architecture:** SQLite owns durable constraints and immutable history; `crates/app-core` owns validation, transition policy, migration orchestration, and typed failures. `ProjectService::open/validate` remain read-only while `ProjectService::upgrade` is the only schema-1 mutation path. `crates/filesystem` supplies a recovery-marked guard so metadata and manifest can be restored together after interruption.

**Tech Stack:** Rust 1.77.2+ workspace, rusqlite 0.40.1 bundled SQLite, SHA-256, canonical JSON Schema generator revision 1, strict TypeScript 5.9, Python 3.12, Cargo test, Vitest, Node test, Ruff, and mypy.

**Source Spec:** `docs/superpowers/specs/2026-07-20-t-0110-job-state-persistence-design.md`

## Global Constraints

- Preserve source data and every existing project/audit record.
- Never mutate a project during `ProjectService::open` or `ProjectService::validate`.
- Add no runtime dependency.
- Use lowercase UUID v7 IDs and UTC RFC 3339 timestamps.
- Store no source rows, dataset values, absolute paths, raw exceptions, or arbitrary JSON in job contracts.
- Execute every material job mutation, `job_event`, and `audit_event` write in one `BEGIN IMMEDIATE` transaction.
- Provide a durable recovery marker and rollback note; never implement destructive downgrade.
- Do not add workers, Python job commands, Tauri job APIs/events, retries, resource preflight, or UI.
- Regenerate generated contracts; never hand-edit them.
- Follow RED -> GREEN -> REFACTOR for every production behavior.

## File Map

| Path | Responsibility |
|---|---|
| `packages/contracts/schemas/job-*.schema.json` | Flat canonical job shapes |
| `packages/contracts/fixtures/job-*.valid.json` | Deterministic contract fixtures |
| `migrations/metadata-sqlite/0002_job_runtime.sql` | Schema-2 tables, checks, triggers, indexes |
| `migrations/metadata-sqlite/0002_job_runtime.rollback.md` | Failure recovery and application rollback rules |
| `crates/filesystem/src/project_upgrade.rs` | Backup, marker, manifest swap, restoration guard |
| `crates/app-core/src/project_upgrade.rs` | Explicit 1 -> 2 migration and manifest hash chain |
| `crates/app-core/src/job.rs` | Job store, queries, transitions, recovery, typed errors |
| `artifacts/*`, `data-contracts/*`, `docs/*`, `README.md` | Traceability and operator guidance |

---

### Task 1: Add flat canonical job contracts

**Files:**
- Create: `packages/contracts/schemas/job-descriptor.schema.json`
- Create: `packages/contracts/schemas/job-enqueue-request.schema.json`
- Create: `packages/contracts/schemas/job-transition-request.schema.json`
- Create: `packages/contracts/schemas/job-progress-update-request.schema.json`
- Create: `packages/contracts/schemas/job-failure-request.schema.json`
- Create: `packages/contracts/fixtures/job-descriptor.valid.json`
- Create: `packages/contracts/fixtures/job-enqueue-request.valid.json`
- Create: `packages/contracts/fixtures/job-transition-request.valid.json`
- Create: `packages/contracts/fixtures/job-progress-update-request.valid.json`
- Create: `packages/contracts/fixtures/job-failure-request.valid.json`
- Modify: `tests/e2e/contracts.test.mjs`
- Modify: `packages/contracts/tests/contract-metadata.test.ts`
- Modify: `packages/contracts/src/index.ts`
- Generated: `packages/contracts/src/generated/job-descriptor.ts`
- Generated: `packages/contracts/src/generated/job-enqueue-request.ts`
- Generated: `packages/contracts/src/generated/job-transition-request.ts`
- Generated: `packages/contracts/src/generated/job-progress-update-request.ts`
- Generated: `packages/contracts/src/generated/job-failure-request.ts`
- Generated: `engine/teratai_engine/generated/job_descriptor.py`
- Generated: `engine/teratai_engine/generated/job_enqueue_request.py`
- Generated: `engine/teratai_engine/generated/job_transition_request.py`
- Generated: `engine/teratai_engine/generated/job_progress_update_request.py`
- Generated: `engine/teratai_engine/generated/job_failure_request.py`
- Generated: `packages/contracts/rust/src/generated/job_descriptor.rs`
- Generated: `packages/contracts/rust/src/generated/job_enqueue_request.rs`
- Generated: `packages/contracts/rust/src/generated/job_transition_request.rs`
- Generated: `packages/contracts/rust/src/generated/job_progress_update_request.rs`
- Generated: `packages/contracts/rust/src/generated/job_failure_request.rs`

**Interfaces:**
- Consumes: generator revision 1 flat primitive support.
- Produces: `JobDescriptor`, `JobEnqueueRequest`, `JobTransitionRequest`, `JobProgressUpdateRequest`, `JobFailureRequest`.

- [ ] **Step 1: Write failing generation and TypeScript shape tests**

Add to `generatedContracts`:

```js
  ["job-descriptor", "job_descriptor", "JobDescriptor"],
  ["job-enqueue-request", "job_enqueue_request", "JobEnqueueRequest"],
  ["job-failure-request", "job_failure_request", "JobFailureRequest"],
  ["job-progress-update-request", "job_progress_update_request", "JobProgressUpdateRequest"],
  ["job-transition-request", "job_transition_request", "JobTransitionRequest"],
```

Add one Vitest case constructing all five types with IDs `00000000-0000-7000-8000-000000000211` through `215`, status `QUEUED`, revision `1`, kind `system.mock_long`, progress `0/100 step`, and safe Indonesian messages. Extend the Node test to require and parse all five matching `.valid.json` fixtures.

- [ ] **Step 2: Verify RED**

```powershell
pnpm test:e2e
pnpm exec vitest run packages/contracts/tests/contract-metadata.test.ts
```

Expected: missing generated modules/public exports.

- [ ] **Step 3: Create the exact flat schema fields**

All five schemas use draft 2020-12, `additionalProperties: true`, descriptions, and only primitive fields:

```text
JobDescriptor required:
job_id:string, project_id:string, kind:string, status:string,
correlation_id:string, revision:integer, created_at:string,
updated_at:string, progress_current:integer

JobDescriptor optional:
started_at:string, finished_at:string, progress_total:integer,
progress_unit:string, progress_phase:string, progress_message:string,
error_code:string, error_message:string, error_retriable:boolean

JobEnqueueRequest required:
job_id:string, kind:string, correlation_id:string
optional: progress_total:integer, progress_unit:string

JobTransitionRequest required:
job_id:string, correlation_id:string, expected_revision:integer

JobProgressUpdateRequest required:
job_id:string, correlation_id:string, expected_revision:integer,
current:integer, phase:string, message:string
optional: total:integer, unit:string

JobFailureRequest required:
job_id:string, correlation_id:string, expected_revision:integer,
error_code:string, error_message:string, error_retriable:boolean
```

Use `$id` values under `https://schemas.teratai.local/job/`. Fixtures omit absent optional properties instead of writing `null`.

- [ ] **Step 4: Generate, export, and verify GREEN**

```powershell
pnpm contracts:generate
pnpm contracts:check
pnpm test:e2e
pnpm exec vitest run packages/contracts/tests/contract-metadata.test.ts
```

Expected: 16 canonical schemas and all focused tests pass.

- [ ] **Step 5: Commit**

```powershell
git add packages/contracts engine/teratai_engine/generated tests/e2e/contracts.test.mjs
git commit -m "feat(contracts): add persistent job contracts"
```

---

### Task 2: Add metadata schema 2 and typed job foundations

**Files:**
- Create: `migrations/metadata-sqlite/0002_job_runtime.sql`
- Create: `migrations/metadata-sqlite/0002_job_runtime.rollback.md`
- Create: `crates/app-core/src/job.rs`
- Modify: `crates/app-core/src/lib.rs`
- Test: `crates/app-core/src/job.rs`

**Interfaces:**
- Consumes: migration 0001 and generated `JobDescriptor`.
- Produces: `JOB_MIGRATION`, `JobError`, `JobErrorKind`, schema-2 tables.

- [ ] **Step 1: Write a failing schema/trigger test**

```rust
#[test]
fn schema_two_enforces_job_shape_and_append_only_events() {
    let connection = migrated_memory_database();
    assert_eq!(user_version(&connection), 2);
    assert!(insert_job_with_status(&connection, "UNKNOWN").is_err());
    insert_job_with_status(&connection, "QUEUED").unwrap();
    insert_job_event(&connection, "job.queued", "QUEUED").unwrap();
    assert!(connection.execute("DELETE FROM job_event", []).is_err());
    assert!(connection.execute("UPDATE job_event SET event_type = 'changed'", []).is_err());
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p teratai-app-core schema_two_enforces_job_shape --locked
```

Expected: missing module and migration.

- [ ] **Step 3: Create `0002_job_runtime.sql`**

Create strict `job` with these exact groups: UUID IDs; FK project; bounded kind; six-value status check; positive revision; UTC timestamps; progress bounds; terminal/finished consistency; FAILED/error consistency. Create strict append-only `job_event` with transition/progress/error snapshots. Add:

```sql
CREATE TRIGGER job_event_prevent_update BEFORE UPDATE ON job_event
BEGIN SELECT RAISE(ABORT, 'job_event is append-only'); END;

CREATE TRIGGER job_event_prevent_delete BEFORE DELETE ON job_event
BEGIN SELECT RAISE(ABORT, 'job_event is append-only'); END;

CREATE INDEX job_status_updated_idx ON job (status, updated_at DESC, job_id DESC);
CREATE INDEX job_correlation_idx ON job (correlation_id, updated_at DESC);
CREATE INDEX job_event_job_sequence_idx ON job_event (job_id, sequence);
CREATE INDEX job_event_correlation_sequence_idx ON job_event (correlation_id, sequence);
PRAGMA user_version = 2;
```

The runner inserts `schema_migrations(version=2,name='job_runtime')` in the same transaction.

- [ ] **Step 4: Add exact typed error surface**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobErrorKind {
    InvalidRequest,
    JobNotFound,
    InvalidTransition,
    RevisionConflict,
    IncompatibleSchema,
    DataIntegrity,
    Database,
    Timestamp,
}

#[derive(Debug)]
pub enum JobError {
    InvalidRequest(String),
    JobNotFound(String),
    InvalidTransition { from: String, to: String },
    RevisionConflict { expected: i64, actual: i64 },
    IncompatibleSchema { expected: i64, actual: i64 },
    DataIntegrity(String),
    Database(rusqlite::Error),
    Timestamp(String),
}
```

Rollback note: no successful in-place downgrade; interrupted migration restores validated schema-1 backups; older applications refuse schema 2 and never delete job/audit history.

- [ ] **Step 5: Verify GREEN and commit**

```powershell
cargo fmt --all
cargo test -p teratai-app-core schema_two_enforces_job_shape --locked
cargo clippy -p teratai-app-core --all-targets --locked -- -D warnings
git add migrations/metadata-sqlite crates/app-core/src
git commit -m "feat(job): add metadata schema two"
```

---

### Task 3: Keep open read-only and create schema-2 projects

**Files:**
- Modify: `crates/app-core/src/lib.rs`
- Modify: `crates/app-core/src/job.rs`
- Test: `crates/app-core/src/lib.rs`

**Interfaces:**
- Produces: new projects at schema 2; read-only validation for schema 1 and 2; safe rejection above 2.

- [ ] **Step 1: Write failing compatibility tests**

```rust
#[test]
fn opens_schema_one_without_mutation() {
    let path = create_schema_one_fixture("readonly-v1");
    let before = control_file_bytes(&path);
    assert_eq!(ProjectService::open(&path).unwrap().metadata_schema_version, 1);
    assert_eq!(control_file_bytes(&path), before);
}

#[test]
fn creates_schema_two_and_rejects_newer_schema() {
    let project = create_project_fixture("schema-v2");
    assert_eq!(project.descriptor.metadata_schema_version, 2);
    assert_eq!(sqlite_user_version(&project.path), 2);
    let newer = create_newer_schema_fixture(3);
    assert!(matches!(ProjectService::open(&newer), Err(ProjectError::IncompatibleProject(_))));
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p teratai-app-core --locked opens_schema_one
cargo test -p teratai-app-core --locked creates_schema_two
```

Expected: exact-version validation rejects v1 and new projects remain v1.

- [ ] **Step 3: Implement ordered migrations and supported range**

```rust
const MIN_METADATA_SCHEMA_VERSION: i64 = 1;
const METADATA_SCHEMA_VERSION: i64 = 2;
const PROJECT_MIGRATIONS: [(i64, &str, &str); 2] = [
    (1, "project_core", PROJECT_MIGRATION),
    (2, "job_runtime", job::JOB_MIGRATION),
];
```

New creation executes both migrations and records both rows in one immediate transaction. Validation requires manifest version, `PRAGMA user_version`, and maximum migration version to agree; version 1 requires only migration 1, version 2 requires both.

- [ ] **Step 4: Verify GREEN and commit**

```powershell
cargo test -p teratai-app-core --locked
cargo clippy -p teratai-app-core --all-targets --locked -- -D warnings
git add crates/app-core/src
git commit -m "feat(project): support metadata schema two"
```

---

### Task 4: Add a recovery-marked filesystem upgrade guard

**Files:**
- Create: `crates/filesystem/src/project_upgrade.rs`
- Modify: `crates/filesystem/src/lib.rs`
- Test: `crates/filesystem/src/project_upgrade.rs`

**Interfaces:**
- Consumes: validated `ProjectLayout`, UUID v7 correlation ID, bounded marker bytes.
- Produces: `begin_project_upgrade` and `ProjectUpgrade` with atomic manifest write, restoration, and commit.

- [ ] **Step 1: Write failing restore/commit tests**

```rust
#[test]
fn uncommitted_upgrade_restores_both_control_files() {
    let layout = project_fixture("restore");
    let before = control_file_bytes(layout.root());
    {
        let upgrade = begin_project_upgrade(&layout, CORRELATION_ID, MARKER).unwrap();
        fs::write(layout.metadata_path(), b"mutated").unwrap();
        upgrade.write_manifest(b"mutated manifest").unwrap();
    }
    assert_eq!(control_file_bytes(layout.root()), before);
}

#[test]
fn committed_upgrade_removes_marker_and_backups() {
    let layout = project_fixture("commit");
    let upgrade = begin_project_upgrade(&layout, CORRELATION_ID, MARKER).unwrap();
    upgrade.write_manifest(b"new manifest").unwrap();
    upgrade.commit().unwrap();
    assert!(!layout.root().join(".project-upgrade-recovery.json").exists());
    assert!(!layout.root().join("recovery/metadata-schema-1.sqlite.backup").exists());
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p teratai-filesystem upgrade --locked
```

Expected: missing upgrade API.

- [ ] **Step 3: Implement the exact guard surface**

```rust
pub fn begin_project_upgrade(
    layout: &ProjectLayout,
    correlation_id: &str,
    marker_contents: &[u8],
) -> Result<ProjectUpgrade, FilesystemError>;

pub struct ProjectUpgrade {
    layout: ProjectLayout,
    marker_path: PathBuf,
    metadata_backup_path: PathBuf,
    manifest_backup_path: PathBuf,
    correlation_id: String,
    committed: bool,
}

impl ProjectUpgrade {
    pub fn write_marker(&self, contents: &[u8]) -> Result<(), FilesystemError>;
    pub fn write_manifest(&self, contents: &[u8]) -> Result<(), FilesystemError>;
    pub fn restore(&mut self) -> Result<(), FilesystemError>;
    pub fn commit(mut self) -> Result<(), FilesystemError>;
}
```

Rules: reject linked/non-regular control or recovery entries; refuse existing markers/backups; sync backups before marker; restore and byte-verify manifest plus metadata; leave recovery artifacts if proof fails; remove marker last on commit. Normal layout validation returns `RecoveryRequired` when the marker or either upgrade backup exists, including a crash that occurred after backup creation but before marker publication.

- [ ] **Step 4: Verify GREEN and commit**

```powershell
cargo fmt --all
cargo test -p teratai-filesystem --locked
cargo clippy -p teratai-filesystem --all-targets --locked -- -D warnings
git add crates/filesystem/src
git commit -m "feat(filesystem): add project upgrade recovery guard"
```

---

### Task 5: Implement explicit project upgrade and manifest authorization chain

**Files:**
- Create: `crates/app-core/src/project_upgrade.rs`
- Modify: `crates/app-core/src/lib.rs`
- Test: `crates/app-core/src/project_upgrade.rs`

**Interfaces:**
- Consumes: filesystem upgrade guard, migrations 0001/0002, manifest hashing, audit table.
- Produces: `ProjectService::upgrade(path, correlation_id)` and authorized manifest-chain validation.

- [ ] **Step 1: Write failing upgrade and failure-restoration tests**

```rust
#[test]
fn upgrade_preserves_identity_and_prior_audit() {
    let path = create_schema_one_fixture("upgrade");
    let before = ProjectService::open(&path).unwrap();
    let audit_before = audit_count(&path);
    let after = ProjectService::upgrade(&path, CORRELATION_ID).unwrap();
    assert_eq!(after.project_id, before.project_id);
    assert_eq!(after.metadata_schema_version, 2);
    assert_eq!(audit_count(&path), audit_before + 1);
    assert_eq!(last_audit_action(&path), "project.metadata_migrated");
    assert_eq!(ProjectService::open(&path).unwrap(), after);
}

#[test]
fn upgrade_failure_restores_v1_or_requires_recovery() {
    let restored = create_schema_one_fixture("restored");
    assert!(upgrade_with_fault(&restored, UpgradeFault::AfterDatabaseCommit).is_err());
    assert_eq!(ProjectService::open(&restored).unwrap().metadata_schema_version, 1);

    let unproven = create_schema_one_fixture("unproven");
    assert!(upgrade_with_fault(&unproven, UpgradeFault::DuringRestoreVerification).is_err());
    assert!(matches!(ProjectService::open(&unproven), Err(ProjectError::Filesystem(FilesystemError::RecoveryRequired(_)))));
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p teratai-app-core --locked upgrade_preserves
cargo test -p teratai-app-core --locked upgrade_failure
```

Expected: missing explicit upgrade orchestration.

- [ ] **Step 3: Implement the approved eight-stage upgrade**

```rust
pub fn upgrade(path: &Path, correlation_id: &str) -> Result<ProjectDescriptor, ProjectError> {
    project_upgrade::upgrade_project(path, correlation_id)
}
```

`upgrade_project` must: validate UUID; read-open v1; return unchanged v2 idempotently; precompute v2 manifest/hash; create marker/backups; run migration 2 plus `project.metadata_migrated` in one immediate transaction; atomically replace manifest; validate schema 2 while the owned marker is present; commit guard only after validation. Test-only fault injection stays under `#[cfg(test)]`.

Use test-only fault points rather than operating-system permission manipulation so restoration tests are deterministic on Windows and CI.

- [ ] **Step 4: Replace single manifest hash validation with chain validation**

Query project-manifest audit actions in sequence. Require first `project.created` with no before hash; every `project.metadata_migrated.before_hash` equals the previous after hash; every after hash is non-empty; final after hash equals current manifest hash. Unknown action/gap/mismatch returns `ProjectError::DataIntegrity`.

- [ ] **Step 5: Verify GREEN and commit**

```powershell
cargo test -p teratai-app-core -p teratai-filesystem --locked
cargo clippy -p teratai-app-core -p teratai-filesystem --all-targets --locked -- -D warnings
git add crates/app-core/src crates/filesystem/src
git commit -m "feat(project): add explicit metadata upgrade"
```

---

### Task 6: Implement enqueue, get, and bounded deterministic list

**Files:**
- Modify: `crates/app-core/src/job.rs`
- Modify: `crates/app-core/src/lib.rs`
- Test: `crates/app-core/src/job.rs`

**Interfaces:**
- Produces: `JobStore::open/enqueue/get/list`, `JobListCursor`, and `JobPage`.

- [ ] **Step 1: Write failing persistence/list tests**

```rust
#[test]
fn jobs_survive_reopen_and_list_is_bounded() {
    let path = create_schema_two_project("list");
    let store = JobStore::open(&path).unwrap();
    for index in 0..3 { store.enqueue_at(&enqueue(index), timestamp(index)).unwrap(); }
    drop(store);
    let reopened = JobStore::open(&path).unwrap();
    let first = reopened.list(2, None).unwrap();
    assert_eq!(first.items.iter().map(|item| item.job_id.clone()).collect::<Vec<_>>(), vec![job_id(2), job_id(1)]);
    let second = reopened.list(2, first.next_cursor).unwrap();
    assert_eq!(second.items[0].job_id, job_id(0));
    assert!(second.next_cursor.is_none());
}

#[test]
fn store_rejects_schema_one_without_mutation() {
    let path = create_schema_one_fixture("job-v1");
    let before = control_file_bytes(&path);
    assert!(matches!(JobStore::open(&path), Err(JobError::IncompatibleSchema { expected: 2, actual: 1 })));
    assert_eq!(control_file_bytes(&path), before);
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p teratai-app-core --locked jobs_survive_reopen
cargo test -p teratai-app-core --locked store_rejects
```

- [ ] **Step 3: Implement exact query API**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobListCursor { pub updated_at: String, pub job_id: String }

#[derive(Debug, Clone, PartialEq)]
pub struct JobPage { pub items: Vec<JobDescriptor>, pub next_cursor: Option<JobListCursor> }

pub struct JobStore { metadata_path: PathBuf, project_id: String }

impl JobStore {
    pub fn open(project_path: &Path) -> Result<Self, JobError>;
    pub fn enqueue(&self, request: &JobEnqueueRequest) -> Result<JobDescriptor, JobError>;
    pub fn get(&self, job_id: &str) -> Result<JobDescriptor, JobError>;
    pub fn list(&self, limit: usize, cursor: Option<JobListCursor>) -> Result<JobPage, JobError>;
}
```

Validate UUIDs, kind `[a-z][a-z0-9._-]{0,119}`, progress bounds, and page size `1..=100`. Enqueue revision 1/QUEUED and append `job.queued` plus audit atomically. List queries `limit + 1` ordered `(updated_at DESC, job_id DESC)` and creates a cursor only when an extra row exists.

Every opened connection enables `PRAGMA foreign_keys = ON`. Generate event IDs without a dependency by combining the timestamp's 48-bit Unix-millisecond prefix with SHA-256 bytes of `(label, job_id, revision, event_type)`, then set UUID version 7 and RFC 4122 variant bits. Use distinct labels for job and audit events; test validity and uniqueness across revisions.

- [ ] **Step 4: Verify GREEN and commit**

```powershell
cargo test -p teratai-app-core --locked
cargo clippy -p teratai-app-core --all-targets --locked -- -D warnings
git add crates/app-core/src
git commit -m "feat(job): persist and query queued jobs"
```

---

### Task 7: Implement transitions, progress, cancellation, concurrency, and recovery

**Files:**
- Modify: `crates/app-core/src/job.rs`
- Test: `crates/app-core/src/job.rs`

**Interfaces:**
- Produces: complete state machine and `recover_interrupted`.

Public methods are:

```rust
pub fn start(&self, request: &JobTransitionRequest) -> Result<JobDescriptor, JobError>;
pub fn succeed(&self, request: &JobTransitionRequest) -> Result<JobDescriptor, JobError>;
pub fn fail(&self, request: &JobFailureRequest) -> Result<JobDescriptor, JobError>;
pub fn update_progress(&self, request: &JobProgressUpdateRequest) -> Result<JobDescriptor, JobError>;
pub fn request_cancellation(&self, request: &JobTransitionRequest) -> Result<JobDescriptor, JobError>;
pub fn complete_cancellation(&self, request: &JobTransitionRequest) -> Result<JobDescriptor, JobError>;
pub fn recover_interrupted(&self, correlation_id: &str) -> Result<Vec<JobDescriptor>, JobError>;
```

- [ ] **Step 1: Write failing transition and revision tests**

```rust
#[test]
fn terminal_state_is_immutable() {
    let store = seeded_store("terminal");
    let running = store.start_at(&transition(1), NOW_1).unwrap();
    let succeeded = store.succeed_at(&transition(running.revision), NOW_2).unwrap();
    assert!(matches!(store.fail_at(&failure(succeeded.revision), NOW_3), Err(JobError::InvalidTransition { .. })));
}

#[test]
fn stale_revision_writes_nothing() {
    let store = seeded_store("revision");
    let before = snapshot_event_audit_counts(&store);
    assert!(matches!(store.start_at(&transition(99), NOW_1), Err(JobError::RevisionConflict { .. })));
    assert_eq!(snapshot_event_audit_counts(&store), before);
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p teratai-app-core terminal_state stale_revision --locked
```

- [ ] **Step 3: Implement one transaction primitive and exact transition table**

Define the internal status model:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobStatus { Queued, Running, Succeeded, Failed, Cancelling, Cancelled }

impl JobStatus {
    #[cfg(test)]
    const ALL: [Self; 6] = [
        Self::Queued, Self::Running, Self::Succeeded,
        Self::Failed, Self::Cancelling, Self::Cancelled,
    ];
}
```

```rust
fn transition_allowed(from: JobStatus, to: JobStatus) -> bool {
    matches!((from, to),
        (JobStatus::Queued, JobStatus::Running)
        | (JobStatus::Queued, JobStatus::Cancelling)
        | (JobStatus::Queued, JobStatus::Failed)
        | (JobStatus::Running, JobStatus::Succeeded)
        | (JobStatus::Running, JobStatus::Failed)
        | (JobStatus::Running, JobStatus::Cancelling)
        | (JobStatus::Cancelling, JobStatus::Cancelled)
        | (JobStatus::Cancelling, JobStatus::Failed))
}
```

All public methods use `WHERE job_id=? AND revision=?`, increment once, then append event and audit only after exactly one row changes.

Add a table-driven test covering all eight allowed pairs and every other pair among the six statuses. Each rejected pair must preserve snapshot/event/audit counts.

- [ ] **Step 4: Write failing progress/cancellation tests**

```rust
#[test]
fn progress_rejects_regression_but_allows_phase_reset() {
    let store = running_store("progress");
    let first = store.update_progress_at(&progress(2, 40, 100, "scan"), NOW_2).unwrap();
    assert!(matches!(store.update_progress_at(&progress(first.revision, 39, 100, "scan"), NOW_3), Err(JobError::InvalidRequest(_))));
    assert_eq!(store.update_progress_at(&progress(first.revision, 0, 10, "write"), NOW_3).unwrap().progress_current, 0);
}

#[test]
fn cancellation_request_is_idempotent() {
    let store = running_store("cancel");
    let running = store.get(JOB_ID).unwrap();
    let cancelling = store.request_cancellation_at(&transition(running.revision), NOW_2).unwrap();
    let counts = snapshot_event_audit_counts(&store);
    assert_eq!(store.request_cancellation_at(&transition(cancelling.revision), NOW_3).unwrap(), cancelling);
    assert_eq!(snapshot_event_audit_counts(&store), counts);
}
```

- [ ] **Step 5: Verify RED, implement progress/cancellation, verify GREEN**

```powershell
cargo test -p teratai-app-core progress_rejects cancellation_request --locked
```

Allow progress only in RUNNING/CANCELLING; enforce total/current and same-phase monotonicity. A phase change may reset current. Repeated cancellation in CANCELLING with current revision returns unchanged without events; completion accepts only CANCELLING.

- [ ] **Step 6: Write failing restart recovery test**

```rust
#[test]
fn recovery_marks_active_jobs_interrupted_once() {
    let store = store_with_running_and_cancelling_jobs("recovery");
    let recovered = store.recover_interrupted_at(RECOVERY_ID, NOW_4).unwrap();
    assert_eq!(recovered.len(), 2);
    assert!(recovered.iter().all(|job| job.status == "FAILED" && job.error_code.as_deref() == Some("INTERRUPTED")));
    let counts = total_event_audit_counts(&store);
    assert!(store.recover_interrupted_at(RECOVERY_ID, NOW_5).unwrap().is_empty());
    assert_eq!(total_event_audit_counts(&store), counts);
}
```

- [ ] **Step 7: Verify RED, implement recovery, run all Rust gates**

```powershell
cargo test -p teratai-app-core recovery_marks_active --locked
cargo fmt --all
cargo test -p teratai-app-core --locked
cargo clippy -p teratai-app-core --all-targets --locked -- -D warnings
```

Recovery handles RUNNING/CANCELLING in one immediate transaction, sets FAILED/INTERRUPTED with `error_retriable=1`, appends `job.interrupted`, leaves QUEUED unchanged, and writes nothing when repeated.

Add an atomicity regression test that installs a test-only abort trigger on `audit_event`, attempts a transition, and proves the job snapshot and `job_event` both roll back when the audit insert fails.

```rust
#[test]
fn audit_failure_rolls_back_snapshot_and_job_event() {
    let store = seeded_store("atomicity");
    install_audit_abort_trigger(&store);
    let before = snapshot_event_audit_counts(&store);
    assert!(matches!(store.start_at(&transition(1), NOW_1), Err(JobError::Database(_))));
    assert_eq!(snapshot_event_audit_counts(&store), before);
    assert_eq!(store.get(JOB_ID).unwrap().status, "QUEUED");
}
```

- [ ] **Step 8: Commit**

```powershell
git add crates/app-core/src/job.rs
git commit -m "feat(job): enforce persistent lifecycle transitions"
```

---

### Task 8: Update artifacts and execute the complete milestone gates

**Files:**
- Modify: `README.md`
- Modify: `artifacts/DATA_DICTIONARY.md`
- Modify: `artifacts/THREAT_MODEL.md`
- Modify: `data-contracts/IPC_CONTRACTS.md`
- Modify: `docs/IMPLEMENTATION_PLAN.md`
- Modify: `docs/CONTEXT_PACK.md`
- Modify: `packages/contracts/README.md`

**Interfaces:**
- Produces: complete traceability and release evidence.

- [ ] **Step 1: Update implementation plan and context pack**

Add:

```markdown
| T-0110 | Persistent job state foundation | `crates/app-core`, `crates/filesystem`, `migrations/metadata-sqlite`, `packages/contracts` | schema-1 projects upgrade explicitly; job state/history survives restart; illegal/concurrent transitions fail atomically; interrupted active jobs remain traceable |
```

Record decisions for explicit migration, snapshot plus append-only history, optimistic revision, idempotent cancellation, FAILED/INTERRUPTED recovery, and excluded executor/UI scope.

- [ ] **Step 2: Update schema/security/contract documentation**

List every implemented `job`/`job_event` column, check, FK, trigger, and index in the data dictionary. Document five flat contracts and transition rules in IPC docs. Add threat controls for migration interruption, linked recovery artifacts, lost updates, immutable history, and bounded safe progress/error text.

- [ ] **Step 3: Update README**

Document explicit schema-1 upgrade, no destructive downgrade, no background execution in T-0110, and:

```powershell
cargo test -p teratai-filesystem -p teratai-app-core --locked
pnpm contracts:check
```

- [ ] **Step 4: Run frozen installs and full gates**

```powershell
$env:CI='true'
$env:UV_CACHE_DIR='D:\DATAAPP\.uv-cache'
pnpm install --frozen-lockfile
uv sync --frozen
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
```

Expected: all commands exit 0 without warnings; 16 canonical schemas; all TypeScript, Node, Rust, and Python suites pass.

- [ ] **Step 5: Audit final diff**

```powershell
git diff --check
git status --short
git diff --stat codex/feat/t-0101-desktop-project-lifecycle...HEAD
rg -n "FIXME|Schema aktif: `1`" README.md artifacts data-contracts docs packages/contracts/README.md
```

Expected: no whitespace errors, unrelated files, placeholders, or stale schema-1-only claims.

- [ ] **Step 6: Commit docs, push, and open stacked draft PR**

```powershell
git add README.md artifacts data-contracts docs packages/contracts/README.md
git commit -m "docs(job): document persistent job foundation"
git push -u origin codex/feat/t-0110-job-state-persistence
```

Open a draft PR against `codex/feat/t-0101-desktop-project-lifecycle`, identify it as stacked on PR #2, include migration/rollback notes, exact test results, risks, and artifacts. Wait for Windows quality to pass.

## Completion Stop Conditions

Stop without committing implementation if schema-1 restoration cannot be proven, open/validate mutate schema-1 bytes, a job snapshot changes without matching job/audit events, terminal states reopen, cancellation reports completion prematurely, a required gate remains red, or the solution needs a new runtime dependency/executor/UI expansion.
