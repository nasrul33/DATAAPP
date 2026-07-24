# Initial Threat Model

## Assets
- Source and derived datasets.
- Personally identifiable and financial data.
- Project metadata, findings, evidence, and exports.
- Analytics integrity and algorithm configuration.

## Trust boundaries
- User ↔ desktop UI.
- React ↔ Tauri commands.
- Rust host ↔ Python sidecar.
- Application ↔ local file system.
- Project ↔ external source/export locations.

## Priority threats and controls
| Threat | Control |
|---|---|
| Arbitrary path access | user-approved absolute target, canonical parent checks, traversal rejection, and linked control-entry rejection |
| Formula/code injection | safe AST; no eval/shell composition |
| Malicious spreadsheet values | treat cells as data; escape exports |
| Sidecar command injection | fixed executable and structured args |
| Data corruption on interruption | sibling staging directory, durable atomic control-file writes, transactional SQLite migration, same-volume publish rename, recovery marker |
| Schema migration interrupted between database and manifest publication | explicit schema-1-to-2 upgrade only; synced byte-for-byte backups plus bounded marker bind backup digest/length and advance through durable stages; failure restores both control files and proves identity/integrity/version/audit chain, otherwise leaves recovery-required evidence intact |
| Recovery artifact substitution through links or stale paths | reject symlink/reparse/non-regular control and recovery entries; hold validated handles; pin metadata identity; verify path/handle identity and backup content proofs before restore, mutation, cleanup, and normal use |
| Lost concurrent job update | independent in-process `JobStore` handles share a serialized pinned-operation capability through transaction commit and identity refresh; positive `expected_revision`, `BEGIN IMMEDIATE`, and CAS make stale peers reach `RevisionConflict` without false corruption or partial snapshot/event writes |
| Malformed persisted job row crosses the trust boundary | every decoded descriptor is revalidated for UUID/status/revision/timeline/cross-field semantics, exact SQLite booleans, byte bounds, and control characters before read, mutation, or recovery use |
| Job snapshot/history divergence | snapshot mutation, one immutable `job_event`, and one hash-linked `audit_event` commit in the same transaction; injected audit failure is tested to roll back all three |
| Job history deletion or rewrite | `job_event` and `audit_event` have unconditional update/delete abort triggers; corrections are new events |
| Progress/error text leaks source values, paths, or raw failures | progress/error unit, token, and text are trimmed, control-character-free, length-bounded (32/120/500), and defined as safe metadata only; raw database/path/data detail never belongs in the contracts |
| Duplicate or unbounded background execution | project-scoped executor accepts only validated `job_id`; bounded FIFO, bounded workers, and an in-process admission set prevent duplicate execution within one executor; cross-process exactly-once is not claimed |
| Resource estimate overflow or aggregate exhaustion | checked arithmetic plus RAII reservations enforce explicit memory/disk/duration policy budgets before `RUNNING`; rejection is audit-backed and never invokes the handler |
| Native handler panic leaks sensitive payload or kills a worker | executor-aware hook is installed before workers, marked handler panic text/location are discarded, `catch_unwind` contains the invocation, and persistence receives only fixed safe `OPERATION_FAILED` metadata |
| Invalid handler cancellation outcome strands a job in `RUNNING` | `HandlerOutcome::Cancelled` is accepted only from trusted `CANCELLING`; an outcome returned from `RUNNING` becomes fixed, non-retriable `FAILED/OPERATION_FAILED` metadata and the worker continues |
| Shutdown abandons queued work or silently drops worker handles | shutdown closes/drains the bounded queue, preserves unstarted snapshots as `QUEUED`, and waits against one configured deadline; every handle stays in a preallocated shared worker slot registered with the reaper before spawn, so timed-out Drop disconnects a payload-free command sender and the prestarted reaper takes/joins remaining handles asynchronously |
| Silent result manipulation | manifest SHA-256, manifest/SQLite identity comparison, SQLite integrity check, immutable runs, append-only events |
| Sensitive data leakage | offline default, masking, explicit export summary |
| Dependency compromise | lockfiles, SBOM, signature/checksum release |

## T-0101 desktop project boundary

- The UI cannot submit an arbitrary unapproved path through a text field. Create and open targets originate from the operating-system save/open dialogs.
- The main window receives only `dialog:allow-open` and `dialog:allow-save`; no broad filesystem plugin permission is granted.
- Rust revalidates UUID v7 correlation, absolute `.teratai` paths, traversal, project integrity, linked control entries, and recovery state before activating a session.
- Native failures are mapped to typed, localized `DesktopError` values. Raw database failures and sensitive absolute paths stay behind the Tauri boundary.
- Recovery-required and corruption failures are non-destructive. The UI provides remediation guidance but never deletes or repairs project data automatically.

## T-0110 persistent job boundary

- Project open/validate accept supported schema 1 or 2 without mutation. Only explicit `ProjectService::upgrade` changes schema 1 to 2; schema 2 has no destructive downgrade.
- Upgrade writes and syncs both schema-1 backups before publishing the marker. The migration, migration-history row, and `project.metadata_migrated` audit event commit together; the authorized manifest replacement is then written atomically and revalidated before recovery artifacts are cleared.
- Every enqueue/transition/progress/recovery mutation is project-scoped, validated against the pinned metadata file, and atomic across the current snapshot, immutable job history, and hash-linked audit history.
- Terminal job states cannot reopen. Cancellation request enters `CANCELLING` without claiming completion, repeated current requests are no-op/idempotent, and only acknowledged cancellation reaches `CANCELLED`. Restart recovery changes only `RUNNING`/`CANCELLING` to retriable `FAILED/INTERRUPTED`; `QUEUED` and terminal jobs are unchanged.
- T-0110 exposes no executor, engine work dispatch, Tauri job command/event, or job UI, so it does not authorize background execution or expand the frontend filesystem boundary.

## T-0111 background executor boundary

- Only handlers registered by native Rust code can execute. Submission carries a bounded UUID v7 `job_id`, not a path, source row, dataset value, arbitrary JSON payload, Python program, or shell command.
- Admission exactly-once is volatile and in-process only. `JobStore` remains the durable state/CAS/audit authority; multi-process or multi-instance coordination is a residual for a later task.
- Handler estimates are checked and reserved against caller-configured memory, disk, and duration budgets. These values are policy ceilings, not measurements of physical memory or free disk; platform probes remain follow-up scope.
- Panic containment covers `JobHandler::run`. A process-wide executor-aware hook delegates unrelated panics to the prior hook, discards marked handler payload/location, and persists only a fixed safe failure. The invariant panic in resource-reservation Drop also uses a fixed non-sensitive message.
- A handler cannot claim cancellation completion by returning `HandlerOutcome::Cancelled` while the trusted snapshot remains `RUNNING`; that contract violation is persisted as a fixed non-retriable `FAILED/OPERATION_FAILED` event so the lifecycle never depends on restart recovery for this defect.
- Cancellation remains persistent and cooperative. A native handler that does not checkpoint cannot be forcibly killed; the observable bounded outcome is `ShutdownTimeout`, with active state retained for T-0110 restart recovery.
- Before any worker spawn, the private reaper receives clones of every preallocated `Arc<WorkerSlot>`. A worker `JoinHandle` is installed in its slot and never sent through the command channel. Graceful shutdown takes/joins handles from those slots only after exit proof.
- After a timed-out Drop, dropping the payload-free command sender disconnects and wakes the prestarted reaper; the reaper takes/joins every remaining slot handle asynchronously. The channel carries only `Stop`, so `Full` or `Disconnected` cannot own or drop a worker handle. This fallback does not make the handler cooperative and does not claim successful shutdown while work remains.
- Engine dispatch, platform resource discovery, Tauri job events/commands, automatic retry, and Job Center UI are not authorized by T-0111.

## Accepted Windows residuals (Option B)

- Stable safe Rust performs `sync_all` on file content, atomic rename, then `sync_all` on the containing directory. Stable `std` does not expose exact `MOVEFILE_WRITE_THROUGH`; the implementation therefore records this best-available durability sequence without claiming that Windows flag.
- Stable safe Rust cannot prove the Windows hardlink count or full by-handle file ID. The implementation mitigates substitution with reparse-point rejection, owned/pinned handles, path-versus-handle metadata identity checks, deny-delete sharing for pinned metadata, and backup content proofs; full hardlink/file-ID proof remains a documented residual requiring a separately audited OS API boundary.
