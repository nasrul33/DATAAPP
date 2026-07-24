# T-0114 Explicit Project Upgrade UI Design

## Objective

Allow a user to explicitly upgrade the active Teratai project from metadata schema 1 to schema 2 so the Job Center can become available, without adding migration side effects to project open, validation, or job access.

## Scope

- one active-project `project_upgrade` Tauri command;
- serialized native project lifecycle mutation;
- safe session activation of the upgraded schema-2 project and its `JobStore`;
- a strict TypeScript project-client method;
- project lifecycle state for upgrade pending, success, retriable failure, and recovery-required failure;
- an accessible confirmation dialog requiring the exact project name;
- replacement of the schema-1 informational Job Center state with an actionable project-upgrade panel;
- deterministic native and desktop regression tests;
- IPC, implementation-plan, and context-pack traceability.

## Non-goals

- a new metadata migration or change to migration 0002;
- automatic upgrade during open, validate, current, or job access;
- schema-2-to-1 downgrade;
- automatic deletion or repair of recovery artifacts;
- arbitrary project paths supplied by the UI;
- background-job execution, Python dispatch, retry orchestration, or operation enqueue;
- dependency or lockfile changes.

## Existing authority

`ProjectService::upgrade(path, correlation_id)` remains the only storage mutation authority. It already:

1. validates a schema-1 project read-only;
2. creates and syncs bounded recovery proofs;
3. commits migration 0002, its migration record, and the hash-linked audit event transactionally;
4. atomically publishes the schema-2 manifest;
5. validates the complete upgraded project;
6. removes recovery artifacts only after durable success;
7. restores byte-identical schema-1 controls on a proven rollback, or retains recovery evidence and blocks normal open when restoration cannot be proven.

T-0114 does not duplicate or weaken this behavior.

## Chosen approach

Expose an active-session command rather than a path-based or implicit upgrade command.

The desktop sends only a canonical `CorrelationRequest`. Native code resolves the trusted absolute project path and identity from `ProjectSession`. This prevents the UI from upgrading a project different from the active project and preserves the existing filesystem trust boundary.

Implicit upgrade on Job Center access is rejected because it would turn a read/control action into an irreversible metadata mutation.

## Native architecture

### Lifecycle serialization

`ProjectSession` gains a private lifecycle-operation mutex. `create`, `open`, `close`, and `upgrade` acquire this guard before performing or publishing a session mutation. `validate`, `current`, and project-scoped job reads remain non-mutating and do not acquire it.

The guard prevents a concurrent open, create, close, or second upgrade from changing the active-session identity between upgrade authorization and session replacement. The existing current-session mutex continues to protect the descriptor and optional `JobStore`.

Lock order is always:

1. lifecycle-operation mutex;
2. current-session mutex only for a bounded snapshot or replacement.

The current-session mutex is not held during filesystem or SQLite work.

### Upgrade command

`project_upgrade(CorrelationRequest)` performs:

1. validate the lowercase UUID v7 request ID;
2. acquire the lifecycle-operation guard;
3. snapshot the active descriptor;
4. reject a missing active project with `VALIDATION_ERROR`;
5. call `ProjectService::upgrade` with the active descriptor path;
6. open a project-scoped `JobStore` with the Tauri lifecycle event sink;
7. replace the active descriptor and store together;
8. return the trusted schema-2 descriptor.

Schema-2 input is idempotent: the core returns the validated descriptor and native activation ensures the `JobStore` is present.

If durable upgrade succeeds but `JobStore` activation fails, the old in-memory session remains visible and the command returns a safe retriable `OPERATION_FAILED`. Retrying is safe because the core upgrade is idempotent; a restart or reopen also discovers the durable schema-2 descriptor. The session never publishes a schema-2 descriptor with a missing job authority.

### Error mapping

`ProjectOperation::Upgrade` receives an explicit mapping:

- invalid request or no active session: `VALIDATION_ERROR`, non-retriable until the user changes the request/session;
- filesystem, database, serialization, or timestamp failure after proven rollback: `OPERATION_FAILED`, retriable only when repeating unchanged input is safe;
- recovery-required, integrity, unsafe layout, linked control file, oversized control file, identity mismatch, or unsupported schema: `PROJECT_CORRUPTED`, non-retriable;
- session lock failure: `OPERATION_FAILED` with restart remediation.

Raw database messages, absolute paths, backup names, and recovery marker contents never cross the desktop boundary.

## Desktop client and lifecycle

`ProjectClient` adds:

```ts
readonly upgrade: () => Promise<ProjectDescriptor>;
```

The Tauri client invokes `project_upgrade` with a newly generated correlation ID and validates the returned descriptor through the existing strict parser.

`ProjectActionStatus` adds `upgrading`. `ProjectLifecycle` adds:

```ts
readonly upgradeProject: () => Promise<boolean>;
```

On success, lifecycle state atomically replaces the active schema-1 descriptor with the returned schema-2 descriptor. The normal active dashboard then mounts the existing Job Center without reloading the window or reopening the project.

On failure, the lifecycle preserves the active schema-1 descriptor and exposes the typed error to the upgrade panel. A retriable error keeps an explicit retry action. `PROJECT_CORRUPTED` switches to the existing non-destructive recovery-required dashboard and does not offer retry.

All create, open, close, and upgrade controls are disabled while `actionStatus === "upgrading"`.

## User experience

### Upgrade panel

An active schema-1 project renders a dedicated project-upgrade panel in place of Job Center. The panel states:

- the current metadata version is 1;
- Job Center requires version 2;
- open and validation remain read-only;
- upgrade changes project control metadata but does not modify source datasets;
- there is no schema downgrade.

The primary action is `Upgrade proyek`.

### Confirmation dialog

The accessible modal uses `role="alertdialog"`, a labelled title and description, deterministic initial focus, keyboard dismissal before submission, and focus return to the trigger.

The user must type the project name with exact case, whitespace, and Unicode code points. No trimming, normalization, or case folding is applied. The confirmation button remains disabled until:

```ts
confirmationValue === project.name
```

The dialog explains:

- upgrade has no downgrade;
- Teratai creates recovery proof before mutation;
- source datasets are not changed;
- the application must remain open while the bounded control-file upgrade completes.

Once submitted, the dialog cannot be dismissed, the input is disabled, and the button shows `Meng-upgrade proyek…`. This operation is not modelled as a cancellable analytical background job because it is a bounded transactional control-file migration with its own durable recovery protocol; interruption is handled by recovery markers rather than unsafe mid-transaction cancellation.

### Success and failure

Success closes the dialog, announces completion through a polite live region, updates the schema metric to version 2, and renders Job Center.

A rollback-safe retriable failure remains in the active schema-1 panel with the typed message, remediation, correlation ID, dismiss, and retry controls. A non-retriable integrity/recovery failure uses the existing full recovery dashboard and never offers delete, automatic repair, or retry.

Browser preview and a missing native runtime keep the existing unavailable/permission state and never emulate upgrade authority.

## Data and security invariants

- Source files and source datasets are never modified.
- Project open, validate, current, and job commands remain migration-free.
- The UI cannot choose or send the upgrade path.
- The active descriptor identity is revalidated by the existing core upgrade.
- Migration 0002 and its rollback notes remain unchanged.
- The audit event written by T-0110 remains the only schema-migration audit record.
- No secret, absolute path, source row, or raw native error is added to UI errors or logs.
- No new runtime dependency, capability permission, canonical schema, or metadata migration is required.

## Testing strategy

### Rust

Add focused `ProjectSession` and command-boundary tests proving:

- schema-1 active session upgrades to schema 2 and exposes a usable `JobStore`;
- schema-2 upgrade is idempotent and activates a missing store;
- missing active session and invalid correlation IDs write nothing;
- concurrent lifecycle mutations cannot replace the authorized project during upgrade;
- rollback-safe core errors map to safe retriable `OPERATION_FAILED`;
- recovery/integrity errors map to non-retriable `PROJECT_CORRUPTED`;
- no raw path or database detail appears in `DesktopError`.

Existing app-core migration, rollback, recovery-marker, and audit-chain tests remain authoritative and must continue to pass.

### TypeScript

Add client and lifecycle tests proving:

- `upgrade` uses the existing descriptor trust boundary;
- success replaces schema 1 with schema 2;
- failure preserves schema 1 when rollback is safe;
- recovery-required switches to the non-destructive error state;
- duplicate submission is disabled while pending.

Add render tests for:

- schema-1 upgrade panel;
- exact-name mismatch;
- enabled exact-name confirmation;
- pending dialog;
- retriable inline error;
- recovery-required dashboard without destructive actions;
- successful schema-2 transition rendering Job Center.

Mocks are limited to the injected native client boundary; storage correctness remains covered by real Rust project fixtures.

## Acceptance criteria

- Opening and validating schema-1 projects remains byte-for-byte read-only.
- Upgrade cannot be invoked without an active project and a valid correlation ID.
- The UI requires an exact project-name confirmation before invocation.
- One successful command upgrades durable storage, replaces the session descriptor, activates `JobStore`, and renders Job Center.
- Safe failures retain data and expose typed actionable states.
- Recovery-required failures retain evidence and expose no destructive or automatic action.
- Strict TypeScript contains no new `any`.
- Rust format, Clippy, tests, TypeScript lint/typecheck/tests, Python gates, contract drift check, and production build pass.
- `docs/IMPLEMENTATION_PLAN.md`, `data-contracts/IPC_CONTRACTS.md`, and `docs/CONTEXT_PACK.md` are updated after implementation.

## Residual scope

T-0114 does not add recovery repair tooling, downgrade, operation enqueue, Python analytics dispatch, automatic job retry, platform resource discovery, or persistent UI preferences.
