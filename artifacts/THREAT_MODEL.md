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
| Silent result manipulation | manifest SHA-256, manifest/SQLite identity comparison, SQLite integrity check, immutable runs, append-only events |
| Sensitive data leakage | offline default, masking, explicit export summary |
| Dependency compromise | lockfiles, SBOM, signature/checksum release |

## T-0101 desktop project boundary

- The UI cannot submit an arbitrary unapproved path through a text field. Create and open targets originate from the operating-system save/open dialogs.
- The main window receives only `dialog:allow-open` and `dialog:allow-save`; no broad filesystem plugin permission is granted.
- Rust revalidates UUID v7 correlation, absolute `.teratai` paths, traversal, project integrity, linked control entries, and recovery state before activating a session.
- Native failures are mapped to typed, localized `DesktopError` values. Raw database failures and sensitive absolute paths stay behind the Tauri boundary.
- Recovery-required and corruption failures are non-destructive. The UI provides remediation guidance but never deletes or repairs project data automatically.
