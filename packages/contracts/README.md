# Cross-language contracts

`packages/contracts/schemas` adalah source of truth untuk kontrak JSON lintas TypeScript, Python, dan Rust. Generated files tidak boleh diedit manual.

## Commands

```powershell
pnpm contracts:generate
pnpm contracts:check
```

`contracts:generate` memvalidasi canonical schemas lalu menulis output deterministik. `contracts:check` bersifat read-only dan gagal jika output hilang atau berbeda dari schema saat ini. Root `pnpm lint` dan CI menjalankan drift check secara otomatis.

## Generated outputs

| Language | Path |
|---|---|
| TypeScript | `packages/contracts/src/generated` |
| Python | `engine/teratai_engine/generated` |
| Rust | `packages/contracts/rust/src/generated` |

Setiap file generated menyimpan path schema dan SHA-256 dari bytes canonical schema. Generator tidak menambahkan timestamp agar output sama pada setiap mesin.

## Generator revision 1

Supported root contract:

- JSON Schema draft 2020-12;
- root `type: object`;
- PascalCase `title`;
- snake_case property names;
- string, integer, number, boolean, dan one-dimensional arrays;
- required dan optional properties;
- `additionalProperties: true` untuk forward-compatible additive fields.

Enum, union, nested object, references, maps, dan runtime validation belum didukung. Generator harus menolak construct tersebut secara eksplisit; jangan menghasilkan tipe parsial atau menebak mapping.

## Compatibility rules

- Additive optional field: compatible pada protocol major yang sama.
- Required field baru, rename, type change, atau field removal: breaking; naikkan protocol major dan sediakan migration/versioned decoder.
- `schema_version` menggunakan semantic version.
- Generated output harus diregenerasi dan contract tests tiga bahasa harus lulus pada setiap perubahan schema.

Crate `teratai-contracts` di `packages/contracts/rust` menjadi dependency bersama bagi runtime Rust. Generated contract tidak ditempatkan di consumer tertentu agar `app-core` dan `engine-host` tidak membentuk dependency cycle.

## Runtime log safety

`RuntimeLogEvent` adalah kontrak trace lintas desktop, native, dan engine. Event hanya membawa metadata operasional yang terdefinisi, UUID v7 correlation ID, dan sequence positif. Source row, nilai dataset, secret, credential, absolute path, serta arbitrary context map dilarang agar observability tidak menjadi jalur kebocoran data.

## Project contracts

`ProjectCreateRequest`, `ProjectOpenRequest`, `CorrelationRequest`, `ProjectManifest`, dan `ProjectDescriptor` membentuk lifecycle project native. `DesktopError` menjadi error envelope aman pada batas Tauri. Contract hanya mendeskripsikan wire/storage shape; validasi UUID v7, absolute `.teratai` path, bounded manifest, schema compatibility, SQLite integrity, fingerprint, serta sanitasi error tetap diwajibkan pada Rust sebelum data dipercaya UI.

## Persistent job contracts

T-0110 menambahkan lima kontrak flat dan additive berikut. Seluruh field optional direpresentasikan sebagai property optional karena generator revision 1 tidak mendukung union dengan `null`.

| Contract | Required fields | Optional fields |
|---|---|---|
| `JobDescriptor` | `job_id`, `project_id`, `kind`, `status`, `correlation_id`, `revision`, `created_at`, `updated_at`, `progress_current` | `started_at`, `finished_at`, `progress_total`, `progress_unit`, `progress_phase`, `progress_message`, `error_code`, `error_message`, `error_retriable` |
| `JobEnqueueRequest` | `job_id`, `kind`, `correlation_id` | `progress_total`, `progress_unit` |
| `JobTransitionRequest` | `job_id`, `correlation_id`, `expected_revision` | - |
| `JobProgressUpdateRequest` | `job_id`, `correlation_id`, `expected_revision`, `current`, `phase`, `message` | `total`, `unit` |
| `JobFailureRequest` | `job_id`, `correlation_id`, `expected_revision`, `error_code`, `error_message`, `error_retriable` | - |

Schema tetap `additionalProperties: true` untuk kompatibilitas additive. Generator tidak memvalidasi enum atau format runtime: Rust wajib memvalidasi UUID v7 lowercase, enam nilai status, revision positif, timestamp UTC, token aman, progress bounds, dan teks aman terbatas sebelum persistence atau konsumsi tepercaya. Kontrak ini belum mengekspos page envelope, executor, command/event Tauri, atau UI job center.
