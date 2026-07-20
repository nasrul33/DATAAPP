# Context Pack — Teratai Analytics Desktop

## Project identity
- Objective: desktop no-code/low-code analytics for audit and data analysis.
- Current phase: planning/foundation.
- Primary OS MVP: Windows.
- Locale: Indonesian UI, Asia/Jakarta display time, locale-safe data parsing.

## Immutable context
- Tauri + React/TypeScript + Python analytics engine.
- Polars/DuckDB/PyArrow for large analytical data.
- SQLite for metadata and audit records.
- Source data immutable.
- Workflow reproducible and explainable.
- Anomaly is not automatically fraud.
- No arbitrary code execution in MVP.

## Business rules
| ID | Rule | Status |
|---|---|---|
| BR-001 | Source datasets are never edited in-place | Confirmed |
| BR-002 | Every derived dataset has operation lineage | Confirmed |
| BR-003 | Every detector result has reason codes | Confirmed |
| BR-004 | Human review controls final anomaly/finding conclusion | Confirmed |
| BR-005 | Long operations must support progress and cancellation | Confirmed |

## Decisions
| ID | Decision | Reason |
|---|---|---|
| ADR-P01 | Tauri rather than Electron | smaller/runtime control |
| ADR-P02 | Python sidecar for analytics | mature data-science ecosystem |
| ADR-P03 | Arrow/file references across large-data boundary | avoid giant JSON and UI memory pressure |
| ADR-P04 | DAG workflow for MVP | reproducibility and simple execution semantics |
| ADR-P05 | schema-first cross-language contracts | prevent drift across TS/Rust/Python |

## Assumptions
| ID | Assumption | Risk if wrong |
|---|---|---|
| AS-001 | Target device has at least 16 GB RAM | performance targets need revision |
| AS-002 | Single-user local project is sufficient for MVP | collaboration architecture moves earlier |
| AS-003 | Windows-first deployment accepted | packaging scope expands if not |

## Traceability sample
| Requirement | Architecture | Backlog | Test |
|---|---|---|---|
| REQ-IMPORT-01 Excel/CSV import | Python importers + job runtime | EPIC-200 | golden import fixtures |
| REQ-ACC-01 accurate profile | Polars + DuckDB reference | EPIC-220 | profile golden suite |
| REQ-TRACE-01 full lineage | operation/version entities | EPIC-100/300 | lineage integration tests |
| REQ-WF-01 rerunnable workflow | DAG execution planner | EPIC-400/410 | restart/rerun e2e |

## Open decisions
- Exact installer/update strategy.
- Encryption priority for MVP versus post-MVP.
- Maximum officially supported dataset size by device class.
- Fuzzy matching algorithm and default threshold after benchmark.

## Handoff instructions
Codex must read AGENTS, Product, Architecture, Primitives, IPC Contracts, current phase, and this context pack before coding. After each significant task, append decisions/deviations and mark completed task IDs; never silently rewrite confirmed decisions.

## Delivery status
| Task | Status | Completed | Evidence |
|---|---|---|---|
| T-0001 | Completed | 2026-07-19 | pnpm, Cargo, and uv workspaces resolve, build, and pass placeholder tests |
| T-0002 | Completed | 2026-07-19 | strict ESLint/TypeScript, Ruff/mypy/Pytest, Clippy, Vitest, and aggregate root gates pass |
| T-0003 | Completed | 2026-07-19 | hardened Windows GitHub Actions workflow runs frozen install, lint, typecheck, test, and build gates |
| T-0004 | Completed | 2026-07-19 | one canonical JSON Schema deterministically generates and round-trips TypeScript, Python, and Rust types |
| T-0005 | Completed | 2026-07-20 | Tauri 2.x opens a tested React/Vite dashboard shell with explicit loading, error, and empty states |
| T-0006 | Completed | 2026-07-20 | Rust launches and supervises the Python 3.12 sidecar while verifying protocol, version, correlation, health, timeout, and typed failures |
| T-0007 | Completed | 2026-07-20 | Canonical structured emitters cover desktop, native, and engine layers; the supervised Rust-Python trace preserves one UUID v7 correlation with deterministic sequence and bounded capture |

## T-0001 decisions and deviations
| ID | Decision/deviation | Reason | Follow-up |
|---|---|---|---|
| DEC-F001 | Keep T-0001 runtime packages dependency-free except TypeScript 5.9.3 | Establish deterministic workspace boundaries without implementing product features or pre-empting dependency review | Add runtime dependencies only in their owning tasks |
| DEC-F002 | Record Tauri 2.x as the desktop target without creating the shell | Desktop shell implementation belongs to T-0005 | T-0005 must add and verify the Tauri 2.x runtime |
| DEC-F003 | Pin Python compatibility to 3.12 and use a uv virtual workspace | Meets the locked engine target while keeping root and engine environments coordinated | Analytics dependencies are added by operation-specific tasks |
| DEC-F004 | Use standard-library Python unittest placeholders for T-0001 | Avoid adding pytest/ruff/mypy before shared quality configuration task T-0002 | T-0002 must activate the standard Python quality gates |
| DEC-F005 | Enforce strict TypeScript compiler defaults from the first scaffold | Prevent permissive types from becoming the workspace baseline | T-0002 may extend lint/test configuration without weakening strictness |

No conflict with confirmed product or architecture decisions was identified during T-0001.

## T-0002 decisions and deviations
| ID | Decision/deviation | Reason | Follow-up |
|---|---|---|---|
| DEC-F006 | Use one shared ESLint flat config and one shared Vitest config from `packages/config` | Prevent configuration drift across TypeScript workspaces | Extend shared config instead of introducing package-local variants |
| DEC-F007 | Make root `lint`, `typecheck`, `test`, and `build` aggregate all active languages | A green root gate must not omit Rust or Python | T-0003 should invoke these root gates in CI and retain explicit frozen install checks |
| DEC-F008 | Enable Ruff and strict mypy from the root uv project, with Pytest discovery covering engine and golden tests | Keep one deterministic Python quality environment and make golden-suite discovery mandatory | Add task-specific fixtures and accuracy assertions with future analytics work |
| DEC-F009 | Allow lifecycle builds only for `esbuild` in pnpm workspace policy | Vitest requires the esbuild native binary while pnpm 11 blocks unapproved lifecycle scripts | Review any future lifecycle dependency individually; do not broaden the allowlist |

T-0002 closes the follow-up recorded in DEC-F004. No product behavior, contract, schema, or runtime boundary changed.

## T-0003 decisions and deviations
| ID | Decision/deviation | Reason | Follow-up |
|---|---|---|---|
| DEC-F010 | Run the foundation CI gate on `windows-latest` | Windows is the primary MVP target and T-0003 requires deterministic Windows/CI behavior | Add Linux coverage when Linux packaging enters an owned task |
| DEC-F011 | Pin every external GitHub Action to an upstream-verified commit SHA | Prevent mutable action tags from changing executed CI code silently | Dependency update work must verify and review replacement SHAs |
| DEC-F012 | Pin Node 24.17.0, Python 3.12.6, Rust 1.97.1, pnpm 11.9.0, and uv 0.11.29 | Align CI with the locally verified foundation and make toolchain drift explicit | Upgrade in a dedicated maintenance change with full gates |
| DEC-F013 | Set CI permissions to `contents: read`, disable persisted checkout credentials, and cancel superseded runs | Reduce token exposure and wasted runner capacity without affecting quality validation | Grant additional permissions only to a separate workflow that owns the operation |
| DEC-F014 | Add a local CI contract test for required commands, immutable action references, and permission invariants | Catch accidental workflow gate removal before the remote workflow runs | T-0003 syntax still receives final validation from GitHub when the workflow is pushed |

T-0003 adds no deployment, release, migration, secret, or product-runtime behavior.

## T-0004 decisions and deviations
| ID | Decision/deviation | Reason | Follow-up |
|---|---|---|---|
| DEC-F015 | Keep canonical machine-readable schemas in `packages/contracts/schemas` and documentation in `data-contracts` | Matches the architecture boundary while retaining human-readable protocol guidance | All future cross-language contract changes start from canonical schema |
| DEC-F016 | Use a dependency-free generator with an explicitly bounded revision-1 JSON Schema subset | Avoid a large code-generation dependency and fail safely on unsupported constructs instead of producing ambiguous types | Expand the subset only with generator tests and three-language fixtures |
| DEC-F017 | Generate directly into TypeScript, Python, and Rust consumer source trees | Ensure generated output is compiled, linted, and tested rather than existing as an unchecked artifact | Consumers must import generated types; manual copies are prohibited |
| DEC-F018 | Fingerprint canonical schema bytes with SHA-256 and omit generation timestamps | Make drift detection deterministic and auditable across machines | Formatting-only schema changes intentionally require regeneration |
| DEC-F019 | Explicitly require additive-field compatibility in canonical schemas | Preserves the confirmed IPC rule that unknown additive fields are tolerated | Breaking changes require protocol major versioning and a migration/versioned decoder |

T-0004 adds only the `ContractMetadata` generation proof. No command, event, job, error code, or product behavior was introduced.

## T-0005 decisions and deviations
| ID | Decision/deviation | Reason | Follow-up |
|---|---|---|---|
| DEC-F020 | Pin the desktop runtime to Tauri 2.11.2 with an explicit main-window capability and restrictive CSP | Establish a deterministic, least-privilege native shell without pre-authorizing product APIs | Add plugin permissions only in the task that owns each native capability |
| DEC-F021 | Use React 19.2.7, Vite 8.1.5, Tailwind CSS 4.3.3, and Lucide as the complete T-0005 UI dependency set | Deliver the approved shell while avoiding state, query, grid, chart, and analytics dependencies before their owning features | Re-evaluate dependencies per feature task; do not preload the target stack |
| DEC-F022 | Render explicit loading, recoverable error, and empty dashboard states while disabling all planned feature actions | Make shell behavior accessible and truthful without implementing project or analytics features | T-0100 owns enabling project creation; T-0006 owns the engine-ready state |
| DEC-F023 | Generate and retain the standard Tauri icon set from one Teratai source asset | Windows resource compilation requires `icon.ico`, while the standard set keeps later packaging targets consistent | Installer branding and signing remain owned by the release task |
| DEC-F024 | Default standalone frontend builds to the Windows Chromium target and select Safari only when Tauri reports macOS | The MVP is Windows-first and Vite 8 cannot downlevel the current bundle to Safari 13 in a host-agnostic standalone build | Re-verify platform targets when macOS packaging enters scope |

T-0005 introduces only the navigational/dashboard shell. Project creation, filesystem access, analytics execution, persistence, IPC commands, and product data remain unimplemented.

## T-0006 decisions and deviations
| ID | Decision/deviation | Reason | Follow-up |
|---|---|---|---|
| DEC-F025 | Move generated Rust contracts into the shared `packages/contracts/rust` crate | Both `app-core` and `engine-host` consume canonical types; placing them in either consumer would create duplication or a future dependency cycle | All Rust runtime layers must depend on `teratai-contracts` instead of copying generated wire structs |
| DEC-F026 | Use bounded newline-delimited JSON over child stdio for lifecycle messages | Provides deterministic framing without network ports or a transport dependency, while enforcing a 64 KiB startup-message limit | Large analytical payloads remain file/Arrow references; T-0006 framing is not permission for tabular JSON |
| DEC-F027 | Inject the Python executable, engine module root, correlation ID, and timeouts through `EngineHostConfig` | Avoid hardcoded installation paths and make startup/shutdown behavior testable across packaged and development environments | Packaging task must resolve the bundled Python executable and engine root before constructing the host |
| DEC-F028 | Verify protocol `1.0`, engine version, Python `3.12.x`, request correlation, `ready` health, and `health` capability before trusting the process | Prevent an incompatible or unhealthy sidecar from accepting analytical work | T-0007 adds correlation-aware runtime logging; job commands remain out of scope |
| DEC-F029 | Keep the sidecar alive after handshake and use stdin EOF for graceful shutdown with a bounded forced-termination fallback | Establishes a supervised process lifecycle without inventing a product command or risking an orphan process | Later engine commands reuse the supervised process and canonical envelopes |

T-0006 adds only lifecycle handshake and typed startup failure behavior. It does not expose a Tauri command, execute analytics, access project files, create jobs, or persist metadata.

## T-0007 decisions and deviations
| ID | Decision/deviation | Reason | Follow-up |
|---|---|---|---|
| DEC-F030 | Define one flat, additive `RuntimeLogEvent` schema for TypeScript, Rust, and Python | Keep cross-layer observability deterministic and prevent consumer-specific event drift | Extend the canonical schema before adding runtime metadata; do not introduce arbitrary context maps |
| DEC-F031 | Generate UUID v7 correlation IDs without a new TypeScript dependency and validate them at Rust/Python boundaries | Preserve time-sortable request identity while minimizing supply-chain surface | Future commands must propagate the initiating correlation ID instead of generating an unrelated child ID |
| DEC-F032 | Reserve engine stdout for IPC and emit structured runtime events on stderr | Prevent logs from corrupting newline-delimited protocol framing | Future engine commands must retain the stdout/stderr separation |
| DEC-F033 | Keep a configurable bounded in-memory trace ordered by sequence and isolate invalid stderr as bounded diagnostics | Avoid unbounded memory growth and prevent malformed messages from entering trusted trace data | Persistent audit/log storage belongs to its owning metadata/audit task |
| DEC-F034 | Enable the `formatting` feature on pinned `time` 0.3.53 already present transitively | Native UTC RFC 3339 formatting is required; no additional resolved Rust package is introduced | Reassess the direct dependency during dedicated dependency maintenance |

T-0007 adds observability only. It does not persist logs, record user/source data, expose an engine Tauri command, execute analytics, create jobs, or enable telemetry.
