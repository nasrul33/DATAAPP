# Data Dictionary — Metadata SQLite

Schema aktif: `2`
Migrations:

- `migrations/metadata-sqlite/0001_project_core.sql`
- `migrations/metadata-sqlite/0002_job_runtime.sql`

Project schema 1 tetap dapat dibuka/divalidasi secara read-only. Perubahan ke schema 2 hanya melalui upgrade eksplisit; tidak ada downgrade in-place yang destruktif.

## `schema_migrations`

| Column | Type | Null | Constraint | Meaning |
|---|---|---|---|---|
| `version` | INTEGER | No | Primary key, `> 0` | Monotonic metadata schema version |
| `name` | TEXT | No | Unique, non-empty | Stable migration name |
| `applied_at` | TEXT | No | UTC ISO-8601 shape | Migration application timestamp |

## `project`

Exactly one row is allowed through `singleton = 1`.

| Column | Type | Null | Constraint | Meaning |
|---|---|---|---|---|
| `singleton` | INTEGER | No | Primary key, equals `1` | Enforces one project identity per database |
| `project_id` | TEXT | No | Unique, UUID-length check | Immutable UUID v7 project identity |
| `name` | TEXT | No | Trimmed length 1–120 | Display name |
| `created_at` | TEXT | No | UTC ISO-8601 shape | Creation time |
| `app_version` | TEXT | No | Non-empty | Creating Teratai version |
| `manifest_schema_version` | TEXT | No | Non-empty | Required manifest format |

## `audit_event`

Append-only table. `audit_event_prevent_update` and `audit_event_prevent_delete` abort mutation; corrections must be new compensating events.

| Column | Type | Null | Constraint | Meaning |
|---|---|---|---|---|
| `sequence` | INTEGER | No | Autoincrement primary key | Durable event order |
| `event_id` | TEXT | No | Unique, UUID-length check | Event identity |
| `actor` | TEXT | No | Non-empty | Actor identifier |
| `action` | TEXT | No | Non-empty | Stable action name |
| `target_type` | TEXT | No | Non-empty | Target entity category |
| `target_id` | TEXT | No | Non-empty | Target identity |
| `before_hash` | TEXT | Yes | — | Prior canonical fingerprint, when applicable |
| `after_hash` | TEXT | Yes | — | Resulting canonical fingerprint |
| `occurred_at` | TEXT | No | UTC ISO-8601 shape | Event time |
| `correlation_id` | TEXT | No | UUID-length check | Cross-layer request correlation |

Initial project creation writes `project.created` with the exact `manifest.json` SHA-256 as `after_hash`. T-0100 contains no destructive migration or downgrade; rollback behavior is documented beside migration 0001.

Indexes `audit_event_correlation_sequence_idx` and `audit_event_target_sequence_idx` support ordered trace and target-history reads without scanning the append-only table.

## `job`

Snapshot mutable terakhir untuk satu pekerjaan. Tabel dibuat sebagai `STRICT`; setiap perubahan material wajib menaikkan `revision` dan berada dalam satu transaksi `BEGIN IMMEDIATE` bersama satu `job_event` serta satu `audit_event`.

| Column | Type | Null | Constraint | Meaning |
|---|---|---|---|---|
| `job_id` | TEXT | No | Primary key; lowercase UUID v7: panjang 36, pola versi/variant benar, hanya `[0-9a-f-]` | Identitas pekerjaan immutable |
| `project_id` | TEXT | No | Lowercase UUID v7; FK ke `project(project_id)` dengan `ON UPDATE RESTRICT ON DELETE RESTRICT` | Project pemilik pekerjaan |
| `kind` | TEXT | No | Panjang 1-120; diawali huruf kecil; hanya `[a-z0-9._-]` | Stable job kind |
| `status` | TEXT | No | Salah satu `QUEUED`, `RUNNING`, `SUCCEEDED`, `FAILED`, `CANCELLING`, `CANCELLED` | Snapshot lifecycle saat ini |
| `correlation_id` | TEXT | No | Lowercase UUID v7 dengan check yang sama seperti `job_id` | Trace mutasi terakhir |
| `revision` | INTEGER | No | `> 0` | Revision optimistic concurrency/CAS |
| `created_at` | TEXT | No | UTC RFC 3339 berbentuk `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, panjang 20-35, jam 00-23, dan tanggal kalender valid melalui `strftime` | Waktu enqueue |
| `started_at` | TEXT | Yes | Jika ada, check UTC/tanggal identik dengan `created_at`; wajib `NULL` saat `QUEUED` | Waktu pertama masuk `RUNNING` |
| `finished_at` | TEXT | Yes | Jika ada, check UTC/tanggal identik; non-null tepat untuk status terminal `SUCCEEDED`, `FAILED`, `CANCELLED` | Waktu terminal |
| `updated_at` | TEXT | No | Check UTC/tanggal identik dengan `created_at` | Waktu mutasi terakhir |
| `progress_current` | INTEGER | No | Default `0`; `>= 0`; tidak boleh melebihi `progress_total` bila total ada | Nilai progress saat ini |
| `progress_total` | INTEGER | Yes | `NULL` atau `> 0` | Total progress yang diketahui |
| `progress_unit` | TEXT | Yes | `NULL` atau trimmed panjang 1-32 | Unit progress aman |
| `progress_phase` | TEXT | Yes | `NULL` atau panjang 1-120, diawali huruf kecil, hanya `[a-z0-9._-]` | Identifier phase aman |
| `progress_message` | TEXT | Yes | `NULL` atau trimmed panjang 1-500 | Pesan progress aman tanpa data sumber/path |
| `error_code` | TEXT | Yes | `NULL` atau panjang 1-120, diawali huruf besar, hanya `[A-Z0-9_]` | Stable safe failure code |
| `error_message` | TEXT | Yes | `NULL` atau trimmed panjang 1-500 | Pesan kegagalan aman tanpa raw error/data/path |
| `error_retriable` | INTEGER | Yes | `NULL`, `0`, atau `1` | Kebijakan retry |

Checks lintas kolom:

- `progress_total IS NULL OR progress_current <= progress_total`.
- `QUEUED` mewajibkan `started_at IS NULL`.
- Status terminal tepatnya `SUCCEEDED|FAILED|CANCELLED` mewajibkan `finished_at`; semua status non-terminal mewajibkan `finished_at IS NULL`.
- `FAILED` mewajibkan ketiga field error terisi; semua status lain mewajibkan ketiganya `NULL`.

Indexes:

- `sqlite_autoindex_job_1 (job_id)` adalah implicit unique index dari `TEXT PRIMARY KEY` pada tabel `STRICT`.
- `job_status_updated_idx (status, updated_at DESC, job_id DESC)` untuk status-filtered/keyset listing deterministik.
- `job_correlation_idx (correlation_id, updated_at DESC)` untuk trace lookup.

## `job_event`

Timeline snapshot append-only untuk setiap enqueue, transition, progress, cancellation, dan recovery. Tabel dibuat sebagai `STRICT`.

| Column | Type | Null | Constraint | Meaning |
|---|---|---|---|---|
| `sequence` | INTEGER | No | `PRIMARY KEY AUTOINCREMENT` | Urutan global durable |
| `event_id` | TEXT | No | Unique; lowercase UUID v7 dengan check panjang/pola/karakter | Identitas event immutable |
| `job_id` | TEXT | No | Lowercase UUID v7; FK ke `job(job_id)` dengan `ON UPDATE RESTRICT ON DELETE RESTRICT` | Pekerjaan pemilik history |
| `event_type` | TEXT | No | Panjang 1-120; diawali huruf kecil; hanya `[a-z0-9._-]` | Stable action, misalnya `job.progressed` |
| `from_status` | TEXT | Yes | `NULL` atau salah satu enam status; app-core memakai `NULL` untuk enqueue | Status sebelum event |
| `to_status` | TEXT | No | Salah satu enam status | Status setelah event |
| `revision` | INTEGER | No | `> 0` | Revision snapshot setelah event |
| `progress_current` | INTEGER | No | `>= 0`; tidak melebihi total bila total ada | Snapshot progress setelah event |
| `progress_total` | INTEGER | Yes | `NULL` atau `> 0` | Snapshot total |
| `progress_unit` | TEXT | Yes | `NULL` atau trimmed panjang 1-32 | Snapshot unit |
| `progress_phase` | TEXT | Yes | `NULL` atau panjang 1-120, diawali huruf kecil, hanya `[a-z0-9._-]` | Snapshot phase |
| `progress_message` | TEXT | Yes | `NULL` atau trimmed panjang 1-500 | Snapshot pesan aman |
| `error_code` | TEXT | Yes | `NULL` atau panjang 1-120, diawali huruf besar, hanya `[A-Z0-9_]` | Snapshot stable failure code |
| `error_message` | TEXT | Yes | `NULL` atau trimmed panjang 1-500 | Snapshot pesan kegagalan aman |
| `error_retriable` | INTEGER | Yes | `NULL`, `0`, atau `1` | Snapshot kebijakan retry |
| `occurred_at` | TEXT | No | UTC RFC 3339 berbentuk `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, panjang 20-35, jam 00-23, tanggal kalender valid | Waktu event |
| `correlation_id` | TEXT | No | Lowercase UUID v7 dengan check panjang/pola/karakter | Trace mutasi |

Checks lintas kolom:

- `progress_total IS NULL OR progress_current <= progress_total`.
- `to_status = FAILED` mewajibkan seluruh field error; semua `to_status` lain mewajibkan seluruh field error `NULL`.

Triggers:

- `job_event_prevent_update`: `BEFORE UPDATE`, selalu `RAISE(ABORT, 'job_event is append-only')`.
- `job_event_prevent_delete`: `BEFORE DELETE`, selalu `RAISE(ABORT, 'job_event is append-only')`.

Indexes:

- `sqlite_autoindex_job_event_1 (event_id)` adalah implicit unique index; `sequence INTEGER PRIMARY KEY` memakai rowid dan tidak membuat index terpisah.
- `job_event_job_sequence_idx (job_id, sequence)` untuk timeline satu job.
- `job_event_correlation_sequence_idx (correlation_id, sequence)` untuk timeline satu trace.

## Migration dan rollback schema 2

Migration 0002 membuat kedua tabel, dua trigger, empat explicit indexes, lalu menetapkan `PRAGMA user_version = 2`; `schema_migrations` versi 2 dan `project.metadata_migrated` ditulis oleh app-core dalam transaksi upgrade yang sama. Upgrade juga mengganti manifest dari metadata schema 1 ke 2 dengan backup/recovery marker dan validasi fingerprint. Tidak ada destructive downgrade: binary lama harus menolak schema 2 dan data job/audit tidak boleh dihapus untuk memaksa kompatibilitas. Detail failure recovery ada di `migrations/metadata-sqlite/0002_job_runtime.rollback.md`.
