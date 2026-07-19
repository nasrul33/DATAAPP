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
| Arbitrary path access | scoped file grants, canonical path checks |
| Formula/code injection | safe AST; no eval/shell composition |
| Malicious spreadsheet values | treat cells as data; escape exports |
| Sidecar command injection | fixed executable and structured args |
| Data corruption on interruption | atomic writes, temp+rename, recovery marker |
| Silent result manipulation | fingerprints, immutable runs, append-only events |
| Sensitive data leakage | offline default, masking, explicit export summary |
| Dependency compromise | lockfiles, SBOM, signature/checksum release |
