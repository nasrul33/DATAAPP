# T-0114 Explicit Project Upgrade UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver an explicit, confirmation-gated schema-1-to-schema-2 project upgrade from the active desktop session, preserving the existing transactional migration authority and rendering Job Center immediately after successful activation.

**Architecture:** The React UI may request an upgrade but never supplies a path. The Tauri boundary serializes lifecycle mutations, resolves the active trusted descriptor, delegates the durable migration exclusively to `ProjectService::upgrade`, activates `JobStore`, and only then publishes the schema-2 session. The UI preserves a schema-1 session on rollback-safe failures and switches to the existing non-destructive recovery state on integrity or recovery failures.

**Tech Stack:** React 19, strict TypeScript 5.9, Vitest 3, Tauri 2, Rust 2024, existing `teratai-app-core`, existing generated contracts, pnpm 11, Cargo, uv/Python 3.12.

## Global Constraints

- Follow `AGENTS.md`, the approved design at `docs/superpowers/specs/2026-07-24-t-0114-explicit-project-upgrade-ui-design.md`, and the architecture boundaries in `docs/ARCHITECTURE.md`.
- Do not add dependencies, permissions, canonical contract schemas, database migrations, or lockfile changes.
- Do not expose or accept a project path in `project_upgrade`; the active native session is the only path authority.
- Keep `project_open`, `project_validate`, `project_current`, and all job reads migration-free.
- Preserve exact project-name confirmation semantics: case, whitespace, and Unicode code points must match without trim, normalization, or case folding.
- Never publish schema 2 in memory unless `JobStore` activation succeeds.
- Preserve the active schema-1 descriptor on rollback-safe failures. Convert recovery/integrity failures to the existing full `PROJECT_CORRUPTED` state with no retry, repair, delete, or downgrade action.
- Treat the upgrade as a bounded transactional control-file operation, not a cancellable analytical background job.
- Run tests before implementation at every task boundary and retain the expected failing output in the task notes or commit message body when useful.

---

## Task 1: Add the serialized native session upgrade boundary

**Files:**

- Modify: `apps/desktop/src-tauri/src/project_commands.rs`
- Modify: `apps/desktop/src-tauri/src/main.rs`
- Test: `apps/desktop/src-tauri/src/project_commands.rs`

### 1.1 Write failing native boundary tests

- [ ] Extend the test-only `ProjectSession` literals with the new lifecycle lock so existing tests describe the intended session shape.
- [ ] Add `upgrade_requires_an_active_project` and assert:

```rust
let session = ProjectSession::default();
let request = CorrelationRequest {
    request_id: REQUEST_ID.to_owned(),
};

let error = session
    .upgrade(&request, Arc::new(TestEventSink))
    .expect_err("missing active project must fail");

assert_eq!(error.code, "VALIDATION_ERROR");
assert!(!error.retriable);
```

- [ ] Add `schema_one_active_session_upgrades_and_activates_job_store`. Create a normal project through `ProjectService::create`, retain its real on-disk path, install a deliberately stale schema-1 descriptor for that same identity as the active session with `job_store: None`, invoke `upgrade`, and assert:

```rust
assert_eq!(upgraded.metadata_schema_version, 2);
assert_eq!(upgraded.project_id, PROJECT_ID);
session
    .job_store(REQUEST_ID)
    .expect("upgrade must activate job store");
```

- [ ] Add `schema_two_upgrade_is_idempotent_and_activates_missing_job_store` using the unchanged descriptor from the same fixture. This separates desktop session activation/idempotency coverage from the real schema-1 durable migration coverage already enforced by `teratai-app-core` migration, rollback, marker, and audit tests.
- [ ] Add focused mapping tests using constructed `ProjectError` values:

```rust
let rollback_safe = map_project_error(
    &ProjectError::Timestamp("D:\\secret\\clock-state".to_owned()),
    REQUEST_ID,
    ProjectOperation::Upgrade,
);
assert_eq!(rollback_safe.code, "OPERATION_FAILED");
assert!(rollback_safe.retriable);
assert!(!rollback_safe.detail.contains("secret"));

let recovery = map_project_error(
    &ProjectError::DataIntegrity("D:\\secret\\metadata.sqlite".to_owned()),
    REQUEST_ID,
    ProjectOperation::Upgrade,
);
assert_eq!(recovery.code, "PROJECT_CORRUPTED");
assert!(!recovery.retriable);
assert!(!recovery.detail.contains("secret"));
```

- [ ] Add a deterministic lifecycle serialization test. Wrap the session in `Arc`, acquire `session.lifecycle` directly from the child test module, spawn a thread that calls `close`, assert `recv_timeout(Duration::from_millis(50))` times out while the guard is held, drop the guard, and assert the close result arrives within one second. This proves a competing mutation cannot replace the authorized session without adding a runtime seam or dependency.

- [ ] Run the focused tests and confirm RED:

```powershell
cargo test -p teratai-desktop project_commands::tests --locked
```

Expected: compilation fails because `ProjectSession::upgrade`, `ProjectOperation::Upgrade`, and the lifecycle guard do not exist.

### 1.2 Implement the lifecycle guard

- [ ] Change the session to own two distinct locks:

```rust
#[derive(Debug, Default)]
pub struct ProjectSession {
    lifecycle: Mutex<()>,
    current: Mutex<Option<ActiveProject>>,
}
```

- [ ] Add:

```rust
fn lock_lifecycle(&self, correlation_id: &str) -> CommandResult<std::sync::MutexGuard<'_, ()>> {
    self.lifecycle
        .lock()
        .map_err(|_| session_error(correlation_id))
}
```

- [ ] Acquire this guard for the complete mutation in `create`, `open`, `close`, and `upgrade`. Do not acquire it in read-only `validate`, `current`, or `job_store`.
- [ ] Keep `activate` as the single helper that constructs `JobStore` before taking the short-lived `current` lock. Never hold `current` while calling the filesystem, SQLite, or app-core migration.
- [ ] Avoid recursive locking by splitting guarded public methods from private helpers:

```rust
fn activate(
    &self,
    descriptor: ProjectDescriptor,
    correlation_id: &str,
    event_sink: Arc<dyn JobEventSink>,
) -> CommandResult<ProjectDescriptor> {
    let job_store = if descriptor.metadata_schema_version == 2 {
        Some(Arc::new(
            JobStore::open_with_event_sink(Path::new(&descriptor.project_path), event_sink)
                .map_err(|error| map_job_activation_error(&error, correlation_id))?,
        ))
    } else {
        None
    };
    let mut current = self
        .current
        .lock()
        .map_err(|_| session_error(correlation_id))?;
    *current = Some(ActiveProject {
        descriptor: descriptor.clone(),
        job_store,
    });
    Ok(descriptor)
}
```

### 1.3 Implement the active-session-only upgrade

- [ ] Add `Upgrade` to `ProjectOperation`.
- [ ] Add the session method:

```rust
pub(crate) fn upgrade(
    &self,
    request: &CorrelationRequest,
    event_sink: Arc<dyn JobEventSink>,
) -> CommandResult<ProjectDescriptor> {
    validate_request_id(&request.request_id)?;
    let _lifecycle = self.lock_lifecycle(&request.request_id)?;
    let descriptor = self
        .current(&CorrelationRequest {
            request_id: request.request_id.clone(),
        })?
        .ok_or_else(|| {
            command_error(
                "VALIDATION_ERROR",
                &request.request_id,
                "no active project is available for upgrade",
                "Tidak ada proyek aktif yang dapat ditingkatkan.",
                "Buka proyek schema versi 1 lalu coba kembali.",
                false,
            )
        })?;
    let upgraded = ProjectService::upgrade(
        Path::new(&descriptor.project_path),
        &request.request_id,
    )
    .map_err(|error| {
        map_project_error(&error, &request.request_id, ProjectOperation::Upgrade)
    })?;
    self.activate(upgraded, &request.request_id, event_sink)
}
```

  During implementation, replace the temporary cloned `CorrelationRequest` call with a private descriptor snapshot helper if Clippy identifies avoidable allocation. The helper must still validate the request once and must not hold `current` during migration.

- [ ] Extend the explicit error match so rollback-safe storage/control-data failures during `Upgrade` return a sanitized retriable `OPERATION_FAILED`, while `RecoveryRequired`, `IncompatibleProject`, `DataIntegrity`, `InvalidLayout`, and `ControlFileTooLarge` remain non-retriable `PROJECT_CORRUPTED`.
- [ ] Ensure a `JobStore` activation error returns the existing safe retriable job error while leaving the old in-memory session unchanged.

### 1.4 Register the Tauri command

- [ ] Add:

```rust
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn project_upgrade(
    request: CorrelationRequest,
    app: AppHandle,
    session: State<'_, ProjectSession>,
) -> CommandResult<ProjectDescriptor> {
    session.upgrade(&request, crate::job_commands::tauri_event_sink(app))
}
```

- [ ] Register `project_commands::project_upgrade` in `tauri::generate_handler!` in `apps/desktop/src-tauri/src/main.rs`.
- [ ] Confirm the command accepts only `CorrelationRequest`; do not add a generated schema because this request already exists.

### 1.5 Verify and commit the native boundary

- [ ] Run:

```powershell
cargo fmt --check
cargo clippy -p teratai-desktop --all-targets --locked -- -D warnings
cargo test -p teratai-desktop project_commands::tests --locked
cargo test -p teratai-app-core project_upgrade --locked
```

Expected: all commands exit `0`; native boundary tests prove missing-session rejection, idempotent activation, serialization, safe error mapping, and no sensitive details.

- [ ] Commit:

```powershell
git add apps/desktop/src-tauri/src/project_commands.rs apps/desktop/src-tauri/src/main.rs
git commit -m "feat(project): add serialized native upgrade command"
```

---

## Task 2: Extend the strict desktop client and lifecycle state machine

**Files:**

- Modify: `apps/desktop/src/project/project-client.ts`
- Modify: `apps/desktop/src/project/use-project-lifecycle.ts`
- Modify: `apps/desktop/tests/app-shell.test.tsx`

### 2.1 Write failing client and state-transition tests

- [ ] In `apps/desktop/tests/app-shell.test.tsx`, add strict pure-state tests for the exported upgrade transition helpers. The helpers exist only to make lifecycle outcomes deterministic and testable without adding a DOM test dependency:

```ts
expect(upgradeSucceeded(schemaTwoDescriptor)).toEqual({
  actionStatus: "idle",
  error: null,
  project: schemaTwoDescriptor,
  status: "active",
});

expect(upgradeFailed(schemaOneState, retriableError)).toEqual({
  actionStatus: "idle",
  error: retriableError,
  project: schemaOneDescriptor,
  status: "active",
});

expect(upgradeFailed(schemaOneState, recoveryError)).toEqual({
  actionStatus: "idle",
  error: recoveryError,
  project: null,
  status: "error",
});
```

- [ ] Test exact-name confirmation as part of Task 3 rather than weakening lifecycle semantics here.
- [ ] Run:

```powershell
pnpm vitest run apps/desktop/tests/app-shell.test.tsx
```

Expected: TypeScript/Vitest fails because `upgradeSucceeded`, `upgradeFailed`, and the `upgrading` action do not exist.

### 2.2 Add the client command

- [ ] Extend `ProjectClient`:

```ts
readonly upgrade: () => Promise<ProjectDescriptor>;
```

- [ ] Add the Tauri implementation:

```ts
async upgrade() {
  return parseProjectDescriptor(
    await invokeProject("project_upgrade", correlationRequest()),
  );
},
```

- [ ] Do not accept a path or name argument. Continue using `parseProjectDescriptor` and `parseDesktopError` as the only native trust boundary.

### 2.3 Add lifecycle upgrade behavior

- [ ] Extend the public types:

```ts
export type ProjectActionStatus =
  | "closing"
  | "creating"
  | "idle"
  | "opening"
  | "selecting"
  | "upgrading";

export interface ProjectLifecycle extends ProjectLifecycleState {
  readonly closeProject: () => Promise<void>;
  readonly createProject: (name: string) => Promise<boolean>;
  readonly dismissError: () => void;
  readonly openProject: () => Promise<boolean>;
  readonly retry: () => Promise<void>;
  readonly upgradeProject: () => Promise<boolean>;
}
```

- [ ] Implement deterministic transition helpers:

```ts
export function upgradeSucceeded(project: ProjectDescriptor): ProjectLifecycleState {
  return {
    actionStatus: "idle",
    error: null,
    project,
    status: "active",
  };
}

export function upgradeFailed(
  current: ProjectLifecycleState,
  error: unknown,
): ProjectLifecycleState {
  const envelope = desktopErrorFrom(error);
  if (envelope.code === "PROJECT_CORRUPTED") {
    return {
      actionStatus: "idle",
      error: envelope,
      project: null,
      status: "error",
    };
  }
  return {
    actionStatus: "idle",
    error: envelope,
    project: current.project,
    status: current.project === null ? "error" : "active",
  };
}
```

- [ ] Extract the existing error-envelope conversion from `errorState` into `desktopErrorFrom(error: unknown): DesktopError`; keep all current fallback text unchanged.
- [ ] Add the hook action:

```ts
const upgradeProject = useCallback(async () => {
  if (client === null) return false;
  setState((current) => ({
    ...current,
    actionStatus: "upgrading",
    error: null,
  }));
  try {
    const project = await client.upgrade();
    setState(upgradeSucceeded(project));
    return true;
  } catch (error: unknown) {
    setState((current) => upgradeFailed(current, error));
    return false;
  }
}, [client]);
```

- [ ] Return `upgradeProject` from the hook.
- [ ] Ensure the state passed to `upgradeFailed` still contains the schema-1 descriptor. No failure path may silently convert schema 1 to schema 2.

### 2.4 Verify and commit the client/lifecycle layer

- [ ] Run:

```powershell
pnpm vitest run apps/desktop/tests/app-shell.test.tsx
pnpm typecheck:ts
pnpm lint:ts
```

Expected: all commands exit `0`; strict TypeScript reports no `any`, and state tests cover success, retriable preservation, and recovery escalation.

- [ ] Commit:

```powershell
git add apps/desktop/src/project/project-client.ts apps/desktop/src/project/use-project-lifecycle.ts apps/desktop/tests/app-shell.test.tsx
git commit -m "feat(project): add desktop upgrade lifecycle"
```

---

## Task 3: Build the exact-name confirmation upgrade surface

**Files:**

- Create: `apps/desktop/src/project/project-upgrade-panel.tsx`
- Modify: `apps/desktop/tests/app-shell.test.tsx`

### 3.1 Write failing UI contract tests

- [ ] Import `ProjectUpgradeDialog`, `ProjectUpgradePanel`, and `projectNameMatchesExactly`.
- [ ] Add exact comparison tests:

```ts
expect(projectNameMatchesExactly("Audit 2026", "Audit 2026")).toBe(true);
expect(projectNameMatchesExactly(" audit 2026", "Audit 2026")).toBe(false);
expect(projectNameMatchesExactly("Audit 2026 ", "Audit 2026")).toBe(false);
expect(projectNameMatchesExactly("audit 2026", "Audit 2026")).toBe(false);
expect(projectNameMatchesExactly("Cafe\u0301", "Café")).toBe(false);
```

- [ ] Render the panel and assert it states version 1, required version 2, source-data immutability, no downgrade, and exposes `Upgrade proyek`.
- [ ] Render the dialog with a mismatched confirmation and assert the submit button is disabled.
- [ ] Render the dialog with an exact confirmation and assert the submit button is enabled.
- [ ] Render the pending dialog and assert `aria-busy="true"`, disabled input/actions, and `Meng-upgrade proyek`.
- [ ] Render retriable and recovery errors separately. Assert retriable markup includes remediation, correlation ID, dismiss, and retry. Assert recovery markup has no retry, delete, repair, or downgrade action.
- [ ] Run:

```powershell
pnpm vitest run apps/desktop/tests/app-shell.test.tsx
```

Expected: module resolution fails because `project-upgrade-panel.tsx` does not exist.

### 3.2 Implement the pure confirmation rule and presentational states

- [ ] Export the exact comparison:

```ts
export function projectNameMatchesExactly(
  confirmationValue: string,
  projectName: string,
): boolean {
  return confirmationValue === projectName;
}
```

- [ ] Define strict props:

```ts
interface ProjectUpgradePanelProps {
  readonly actionStatus: ProjectLifecycle["actionStatus"];
  readonly error: DesktopError | null;
  readonly onDismissError: () => void;
  readonly onUpgrade: () => Promise<boolean>;
  readonly project: ProjectDescriptor;
}

interface ProjectUpgradeDialogProps {
  readonly confirmationValue: string;
  readonly onCancel: () => void;
  readonly onConfirmationChange: (value: string) => void;
  readonly onSubmit: () => void;
  readonly pending: boolean;
  readonly projectName: string;
}
```

- [ ] `ProjectUpgradePanel` owns only transient dialog state: open/closed, exact confirmation value, and a polite success announcement. Durable project/error state remains in `useProjectLifecycle`.
- [ ] The panel primary action is disabled whenever `actionStatus !== "idle"`.
- [ ] The error area must render only the safe `DesktopError` fields: `message`, optional `remediation`, `field_errors`, and `correlation_id`. Never render `detail`.

### 3.3 Implement accessible modal behavior

- [ ] Use `role="alertdialog"`, `aria-modal="true"`, `aria-labelledby`, and `aria-describedby`.
- [ ] Use an input ref for deterministic initial focus and a trigger ref for focus restoration.
- [ ] Handle `Escape` only while `pending === false`.
- [ ] Disable the input, cancel action, close action, and submit action during pending work.
- [ ] Set `autoComplete="off"` and `spellCheck={false}` on the confirmation input.
- [ ] Enable submit only when:

```ts
const confirmed = projectNameMatchesExactly(
  confirmationValue,
  projectName,
);
```

- [ ] Submit through:

```ts
const upgraded = await onUpgrade();
if (upgraded) {
  setDialogOpen(false);
  setConfirmationValue("");
  setAnnouncement("Upgrade proyek selesai. Metadata schema sekarang versi 2.");
}
```

- [ ] Do not close or clear confirmation after a retriable failure. This preserves an explicit user-controlled retry.
- [ ] The modal copy must explicitly state: no downgrade, recovery proof is created, source datasets are unchanged, and the application must remain open during the bounded migration.

### 3.4 Verify and commit the UI component

- [ ] Run:

```powershell
pnpm vitest run apps/desktop/tests/app-shell.test.tsx
pnpm typecheck:ts
pnpm lint:ts
```

Expected: all commands exit `0`; SSR tests prove every static state and the pure helper proves exact Unicode semantics.

- [ ] Commit:

```powershell
git add apps/desktop/src/project/project-upgrade-panel.tsx apps/desktop/tests/app-shell.test.tsx
git commit -m "feat(project): add explicit upgrade confirmation UI"
```

---

## Task 4: Integrate schema-aware dashboard behavior

**Files:**

- Modify: `apps/desktop/src/app/app-shell.tsx`
- Modify: `apps/desktop/tests/app-shell.test.tsx`
- Modify: `apps/desktop/src/job/job-center.tsx`
- Modify: `apps/desktop/tests/job-center.test.tsx`

### 4.1 Replace the informational schema-1 assertion with an actionable flow

- [ ] Update the existing schema-1 `AppShell` test to assert:

```ts
expect(markup).toContain("Upgrade proyek diperlukan");
expect(markup).toContain("Upgrade proyek");
expect(markup).toContain("Metadata schema versi 1");
expect(markup).not.toContain("tidak di-upgrade otomatis");
expect(markup).not.toContain("Belum ada pekerjaan");
```

- [ ] Add an upgrading-state assertion:

```ts
const markup = renderToStaticMarkup(
  <AppShell
    lifecycle={lifecycle({
      actionStatus: "upgrading",
      project: schemaOneDescriptor,
      status: "active",
    })}
  />,
);
expect(markup).toContain("Meng-upgrade");
expect(markup).toContain("disabled");
```

- [ ] Extend the test lifecycle factory with:

```ts
upgradeProject: () => Promise.resolve(false),
```

- [ ] Run:

```powershell
pnpm vitest run apps/desktop/tests/app-shell.test.tsx
```

Expected: the old informational Job Center markup causes the new assertions to fail.

### 4.2 Wire the upgrade panel into the active dashboard

- [ ] Pass lifecycle upgrade state/actions from `AppShell` to `ActiveProjectDashboard`:

```tsx
<ActiveProjectDashboard
  actionStatus={lifecycle.actionStatus}
  error={lifecycle.error}
  jobClient={jobClient}
  onClose={() => void lifecycle.closeProject()}
  onDismissError={lifecycle.dismissError}
  onUpgrade={lifecycle.upgradeProject}
  project={lifecycle.project}
/>
```

- [ ] Render exactly one schema-dependent lower panel:

```tsx
{project.metadata_schema_version === 1 ? (
  <ProjectUpgradePanel
    actionStatus={actionStatus}
    error={error}
    onDismissError={onDismissError}
    onUpgrade={onUpgrade}
    project={project}
  />
) : (
  <JobCenter client={jobClient} project={project} />
)}
```

- [ ] Preserve the schema metric. After success the returned descriptor changes it to `Versi 2` and React mounts Job Center without reload.
- [ ] Keep close disabled for every non-idle action, including `upgrading`.

### 4.3 Remove contradictory informational copy

- [ ] Replace the direct `JobCenterUpgrade` fallback copy with a defensive non-actionable statement that does not claim the product lacks an upgrade flow. It remains a guard for callers that bypass `AppShell`; it must not invoke native commands itself.
- [ ] Update `job-center.test.tsx` to expect the defensive fallback, while `app-shell.test.tsx` remains authoritative for the actionable flow.
- [ ] Do not move project lifecycle authority into `JobCenter`.

### 4.4 Verify and commit dashboard integration

- [ ] Run:

```powershell
pnpm vitest run apps/desktop/tests/app-shell.test.tsx apps/desktop/tests/job-center.test.tsx
pnpm typecheck:ts
pnpm lint:ts
```

Expected: all commands exit `0`; schema 1 renders upgrade UI, schema 2 renders Job Center, and no old contradictory copy remains.

- [ ] Commit:

```powershell
git add apps/desktop/src/app/app-shell.tsx apps/desktop/src/job/job-center.tsx apps/desktop/tests/app-shell.test.tsx apps/desktop/tests/job-center.test.tsx
git commit -m "feat(project): integrate schema upgrade dashboard"
```

---

## Task 5: Update traceability and context artifacts

**Files:**

- Modify: `docs/IMPLEMENTATION_PLAN.md`
- Modify: `docs/CONTEXT_PACK.md`

### 5.1 Record completed scope and decisions

- [ ] Add a T-0114 implementation-plan entry that records:
  - explicit active-session-only upgrade;
  - exact-name confirmation;
  - serialized lifecycle mutations;
  - schema-2 activation only after `JobStore` opens;
  - rollback-safe retry versus recovery-required behavior;
  - no new migration, dependency, contract schema, or capability.
- [ ] Update `docs/CONTEXT_PACK.md` with:
  - T-0114 status and verification evidence;
  - replacement of residual schema-upgrade UI gaps from DEC-F071/DEC-F079;
  - a new decision ID for active-session-only path authority and lifecycle serialization;
  - a new decision ID for exact project-name confirmation;
  - remaining scope: recovery repair tooling, operation enqueue, Python dispatch, automatic retry runner, platform resource discovery, and persistent UI preferences.
- [ ] Preserve all historical decisions and planning artifacts; append or amend status without deleting prior evidence.

### 5.2 Validate documentation and commit

- [ ] Run:

```powershell
rg -n "T-0114|project_upgrade|exact|schema-1|schema 1|recovery" docs/IMPLEMENTATION_PLAN.md docs/CONTEXT_PACK.md
git diff --check
```

Expected: T-0114 is traceable in both artifacts and `git diff --check` exits `0`.

- [ ] Commit:

```powershell
git add docs/IMPLEMENTATION_PLAN.md docs/CONTEXT_PACK.md
git commit -m "docs(project): record explicit schema upgrade delivery"
```

---

## Task 6: Run the complete repository quality gates

**Files:**

- Verify only; modify files only if a gate reveals a defect within T-0114 scope.

### 6.1 Run deterministic installs and generated-contract verification

- [ ] Run:

```powershell
pnpm install --frozen-lockfile
uv sync --frozen
pnpm contracts:check
```

Expected: all commands exit `0`; no lockfile or generated-contract diff appears.

### 6.2 Run all TypeScript, Rust, and Python gates

- [ ] Run:

```powershell
pnpm lint
pnpm typecheck
pnpm test
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
uv run ruff check engine tests/golden
uv run mypy
uv run pytest
pnpm build
```

Expected: every command exits `0`. `pnpm test` includes unit, repository e2e, Rust, and Python coverage; the explicit Cargo/uv commands provide independently reportable evidence required by `AGENTS.md`.

### 6.3 Audit scope and repository state

- [ ] Run:

```powershell
git diff origin/main...HEAD --check
git status --short
git diff origin/main...HEAD -- pnpm-lock.yaml uv.lock Cargo.lock packages/contracts data-contracts
```

Expected:

- no whitespace errors;
- only intended T-0114 files are changed;
- no uncommitted production changes remain;
- no dependency lockfile, canonical contract, or migration changes are present.

- [ ] If verification-driven fixes were required, stage only the explicit files shown by `git status --short`, inspect the staged diff with `git diff --cached`, and commit them separately with:

```powershell
git commit -m "fix(project): satisfy upgrade quality gates"
```

### 6.4 Prepare delivery evidence

- [ ] Record the exact command, exit code, and concise result for every quality gate in the final handoff.
- [ ] Report changed files, architectural decisions, security/data-safety invariants, tests, residual risks, and remaining follow-up scope.
- [ ] Use `superpowers:verification-before-completion` before claiming completion or pushing.
- [ ] Use `superpowers:finishing-a-development-branch` after all implementation commits and gates pass to choose the integration path.
