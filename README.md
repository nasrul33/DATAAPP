# Teratai Analytics Desktop

Fondasi monorepo untuk aplikasi desktop analitik data yang offline-first, reproducible, explainable, dan menjaga kepemilikan data lokal.

Versi fondasi: 0.1.0
Target utama: Windows, Tauri 2.x, Python 3.12

## Prasyarat

Toolchain yang telah diverifikasi untuk T-0001 sampai T-0111:

- Node.js 24.17.0 atau kompatibel dengan `>=24.0.0`.
- pnpm 11.9.0.
- Rust stable dengan Cargo dan komponen `rustfmt` serta `clippy` (diverifikasi pada Rust 1.97.1).
- uv 0.11.29 atau lebih baru yang kompatibel.
- Python 3.12; uv akan memilih atau menyiapkan interpreter yang sesuai.
- Microsoft Edge WebView2 Runtime dan Microsoft C++ Build Tools untuk menjalankan/build target Tauri di Windows.

Versi CI dikunci melalui `.node-version`, `.python-version`, `rust-toolchain.toml`, field `packageManager`, dan `tool.uv.required-version`. Perubahan versi harus dilakukan secara eksplisit dan diverifikasi dengan seluruh quality gate.

## Setup lokal

Jalankan dari root repository:

```powershell
pnpm install --frozen-lockfile
cargo check --workspace --locked
uv sync --frozen
```

Perintah tersebut telah diverifikasi pada Windows 11 dengan Python 3.12.6. Lockfile `pnpm-lock.yaml`, `Cargo.lock`, dan `uv.lock` harus ikut disimpan agar resolusi workspace konsisten.

## Menjalankan desktop shell

Jalankan UI di browser untuk pengembangan frontend:

```powershell
pnpm --dir apps/desktop dev
```

Jalankan aplikasi native Tauri 2.x:

```powershell
pnpm --dir apps/desktop desktop:dev
```

Build frontend atau executable desktop secara eksplisit:

```powershell
pnpm --filter @teratai/desktop build
pnpm --dir apps/desktop desktop:build
```

Lifecycle proyek tersedia ketika aplikasi dijalankan melalui Tauri. Preview browser sengaja menampilkan state izin karena file picker native tidak tersedia. Dataset, workflow, temuan, dan ekspor tetap nonaktif sampai task pemilik fiturnya diimplementasikan.

## Quality gates

### Seluruh workspace

```powershell
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

Root scripts mengorkestrasi quality gate TypeScript, Rust, dan Python. Kegagalan salah satu ekosistem menghentikan command dengan exit code non-zero.

| Root command | Gate yang dijalankan |
|---|---|
| `pnpm lint` | ESLint strict type-aware, Cargo fmt/Clippy, Ruff |
| `pnpm typecheck` | TypeScript strict, Cargo check, mypy strict |
| `pnpm test` | Vitest, workspace e2e, Cargo test, Pytest |
| `pnpm build` | TypeScript build, Cargo build, Python bytecode validation |

### Rust workspace

```powershell
cargo fmt --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

### Python workspace

```powershell
uv sync --frozen
uv run ruff check engine tests/golden
uv run mypy
uv run pytest
```

Python quality tools dikunci melalui `uv.lock`. Pytest menemukan engine tests dan golden tests dari konfigurasi root `pyproject.toml`.

## Continuous integration

Workflow `.github/workflows/quality.yml` berjalan pada `windows-latest` untuk:

- setiap pull request;
- setiap push ke `main`;
- eksekusi manual melalui `workflow_dispatch`.

Pipeline menggunakan permission `contents: read`, action yang dipin ke commit SHA, frozen pnpm/uv installs, cache berbasis lockfile, timeout 30 menit, dan concurrency cancellation. Urutan gate:

```text
checkout → setup toolchains → frozen installs → lint → typecheck → test → build
```

CI tidak memerlukan secret atau environment variable produk pada fase foundation.

## Contract generation

Canonical JSON Schemas berada di `packages/contracts/schemas`. Setelah mengubah schema:

```powershell
pnpm contracts:generate
pnpm contracts:check
pnpm test
```

Generator dependency-free menghasilkan TypeScript, Python, dan Rust dari source yang sama. CI menolak generated output yang hilang atau stale. Detail supported subset dan compatibility rules tersedia di `packages/contracts/README.md`.

## Engine sidecar handshake

T-0006 menyediakan host Rust yang meluncurkan Python 3.12 melalui executable dan module root yang diberikan secara eksplisit. Host memverifikasi protocol `1.0`, engine version, Python version, health, capability, correlation ID, timeout, dan typed error envelope.

Verifikasi handshake terisolasi:

```powershell
cargo test -p teratai-engine-host --locked
uv run pytest engine/tests/test_sidecar_handshake.py
```

Sidecar menggunakan newline-delimited JSON maksimal 64 KiB per lifecycle message. Proses tetap hidup setelah handshake dan berhenti secara graceful ketika stdin ditutup; forced termination hanya digunakan setelah shutdown timeout.

## Runtime logging dan correlation ID

T-0007 menyediakan event log terstruktur yang sama untuk layer desktop, native, dan engine. Setiap trace menggunakan UUID v7, sequence positif, timestamp UTC, dan metadata aman. Python menulis log ke stderr agar stdout tetap eksklusif untuk protokol IPC; Rust memvalidasi dan menyimpan trace in-memory dengan batas yang dapat dikonfigurasi.

Verifikasi trace lintas layer:

```powershell
pnpm test:unit
cargo test -p teratai-engine-host --locked
uv run pytest engine/tests/test_runtime_logging.py engine/tests/test_sidecar_handshake.py
```

Log tidak boleh memuat source row, nilai dataset, secret, credential, atau absolute path. Persistensi audit/log dan integrasi command Tauri belum diaktifkan pada tahap ini.

## Project storage core

T-0100 menyediakan fondasi native `project.create/open/validate`: direktori `.teratai` dibuat melalui staging sibling, recovery marker, migration SQLite transaksional, manifest maksimal 64 KiB, SHA-256 fingerprint, dan publish rename satu volume. Project yang valid memiliki struktur berikut:

```text
project.teratai/
├── manifest.json
├── metadata.sqlite
├── data/source
├── data/derived
├── data/cache
├── workflows
├── findings
├── exports
├── attachments
├── logs
└── recovery
```

Verifikasi lifecycle storage secara terisolasi:

```powershell
cargo test -p teratai-filesystem -p teratai-app-core --locked
```

## Persistent job state

T-0110 menaikkan metadata SQLite project baru ke schema 2 dan menyediakan upgrade eksplisit untuk project schema 1 melalui `ProjectService::upgrade`. `project.open` dan `project.validate` tetap read-only: keduanya tidak melakukan migration terselubung. Upgrade membuat backup schema 1 dan recovery marker yang mengikat digest/panjang backup serta stage aktual, menerapkan migration SQLite dalam transaksi immediate, menulis manifest schema 2 secara atomik, lalu memvalidasi kembali identitas, fingerprint, integrity, dan rantai audit sebelum artifact recovery dibersihkan.

Schema 2 menyimpan snapshot `job`, riwayat `job_event` append-only, dan `audit_event` terkait dalam transaksi yang sama. State machine menggunakan optimistic revision/CAS, termasuk antar-handle `JobStore` dalam proses yang sama; shared operation guard menutup window commit-ke-identity-refresh agar peer reader tidak melihat false corruption. Progress hanya dapat berubah pada `RUNNING`/`CANCELLING`; permintaan cancellation bersifat kooperatif dan idempotent; recovery restart mengubah pekerjaan aktif menjadi `FAILED` dengan kode `INTERRUPTED` yang retriable. Setiap descriptor hasil baca divalidasi kembali sebagai data tidak tepercaya sebelum digunakan atau dikembalikan. Snapshot dan history dapat dibuka kembali setelah restart.

Tidak ada destructive downgrade dari schema 2 ke schema 1. Binary lama harus menolak schema 2; rollback mempertahankan project, job, dan seluruh audit history, lalu menggunakan binary yang mendukung schema 2. Jika upgrade gagal sebelum tervalidasi, backup schema 1 dipulihkan byte-for-byte atau project tetap recovery-required.

T-0110 belum menjalankan pekerjaan di background dan tidak menambahkan executor, command/event Tauri untuk job, resource preflight, retry orchestration, maupun UI job center. Verifikasi foundation ini secara terisolasi:

```powershell
cargo test -p teratai-filesystem -p teratai-app-core --locked
pnpm contracts:check
```

## Background job executor core

T-0111 menyediakan `JobExecutor` Rust project-scoped dengan worker dan submission queue bounded. Caller native mempersistenkan job melalui `JobStore`, lalu submit hanya `job_id`; handler terdaftar berjalan di background, menulis progress melalui CAS, mengamati cancellation persisten pada checkpoint, dan menghasilkan lifecycle plus audit history yang tetap dapat dibaca setelah reopen. Resource estimate ditolak sebelum handler berjalan jika melampaui budget memory/disk/duration yang dikonfigurasi. Handler error dan panic dipetakan ke failure metadata yang aman. Shutdown memakai deadline eksplisit; setiap handle tetap di preallocated shared worker slot, dan setelah timed-out Drop, sender disconnection membangunkan private reaper untuk mengambil serta join handle tersisa secara asynchronous.

Outcome cancellation dari handler hanya diterima jika snapshot sudah `CANCELLING`. Outcome cancellation tanpa permintaan aktif dipersistenkan sebagai `FAILED/OPERATION_FAILED` yang aman dan non-retriable, sehingga defect handler tidak meninggalkan job `RUNNING`. Test executor juga membuktikan overflow/aggregate resource rejection serta kontinuitas revision dan before/after audit hash pada jalur success, cancellation, failure, panic, preflight, dan restart recovery.

Verifikasi executor secara terisolasi dari root repository:

```powershell
cargo test -p teratai-app-core job_executor::tests --locked
```

T-0111 sendiri adalah API native Rust. Exactly-once admission berlaku hanya di dalam satu instance executor. Handler native wajib checkpoint secara bounded; handler non-kooperatif tidak dapat dipaksa berhenti dan akan terlihat sebagai `ShutdownTimeout`.

## Typed desktop job IPC

T-0112 menambahkan command Tauri schema-first `job_get`, `job_list`, dan `job_cancel` yang selalu dibatasi ke project aktif. Listing menggunakan keyset cursor dan limit `1..=100`; cancellation membawa revision persisten agar stale caller gagal aman. Project schema 1 tetap dapat dibuka read-only, tetapi command job mengembalikan `PROJECT_UPGRADE_REQUIRED` dan tidak menjalankan migration otomatis.

Setiap mutasi `JobStore` yang sudah commit dapat mengirim event best-effort pada channel `job:lifecycle`. Sequence event sama dengan revision snapshot dan payload selalu membawa `JobDescriptor` durable. Event bukan source of truth; client memulihkan event yang terlewat melalui `job_get` atau `job_list`. Client TypeScript memvalidasi ulang descriptor, page, cursor, event, dan error envelope sebelum dipercaya UI.

Verifikasi IPC job secara terisolasi:

```powershell
pnpm contracts:check
pnpm typecheck:ts
pnpm test:unit
cargo test -p teratai-app-core -p teratai-desktop --locked
```

T-0112 belum menambahkan Python engine dispatch, operation enqueue command, automatic retry, platform memory/free-disk probe, project upgrade UI, atau Job Center UI.

## Desktop project lifecycle

T-0101 menghubungkan project core ke UI melalui command Tauri typed `project_create`, `project_open`, `project_validate`, `project_current`, dan `project_close`. Buat/buka proyek selalu memakai dialog sistem; capability main window dibatasi ke `dialog:allow-open` dan `dialog:allow-save`.

Jalankan lifecycle native dari root repository:

```powershell
pnpm install --frozen-lockfile
pnpm --dir apps/desktop desktop:dev
```

Create/open menampilkan loading dan mencegah aksi ganda. Permission, error, empty, active, dan recovery/corruption memiliki state terpisah. Recovery bersifat non-destruktif: aplikasi tidak menghapus, menimpa, atau memperbaiki project secara otomatis. Session aktif hanya disimpan di memori proses; close tidak mengubah file project.

## Prinsip produk

- Desktop-first, offline-first, dan local data ownership.
- No-code untuk analisis umum; low-code untuk formula dan rule.
- Setiap hasil harus reproducible, explainable, dan traceable.
- Data sumber immutable; transformasi menghasilkan turunan.
- Anomali bukan otomatis fraud.
- Akurasi analitik harus dibuktikan dengan golden dataset dan cross-check.

## Stack terkunci untuk MVP

- Desktop shell: Tauri 2.x + Rust.
- UI: React + TypeScript + Vite + Tailwind CSS.
- Server state/job state: TanStack Query.
- Local UI state: Zustand secukupnya.
- Analytics engine: Python 3.12, Polars, DuckDB, PyArrow, SciPy, scikit-learn.
- Metadata store: SQLite.
- Charts: Plotly.
- Testing: Vitest, React Testing Library, Playwright, Pytest, dan Cargo test.
- Monorepo: pnpm workspaces + uv untuk Python.

## Struktur workspace

```text
apps/desktop              Tauri 2.x + React/Vite desktop shell
crates/app-core           Rust application orchestration boundary
crates/filesystem         Rust safe filesystem boundary
crates/engine-host        Rust Python sidecar lifecycle/IPC boundary
crates/secure-store       Rust secure storage boundary
engine/teratai_engine     Python analytics engine package
packages/contracts        Cross-language contract source boundary
packages/ui               Reusable UI boundary
packages/workflow         Workflow DAG boundary
packages/config           Shared TypeScript foundation
tests/e2e                 End-to-end workspace tests
tests/fixtures            Shared deterministic fixtures
tests/golden              Analytics golden test boundary
```

## Urutan baca wajib

1. `AGENTS.md`
2. `docs/PRODUCT.md`
3. `docs/ARCHITECTURE.md`
4. `docs/PRIMITIVES.md`
5. `data-contracts/IPC_CONTRACTS.md`
6. bagian task terkait pada `docs/IMPLEMENTATION_PLAN.md`
7. `artifacts/ARTIFACT_CATALOG.md`
8. `skills/SKILLS.md`
9. `docs/CONTEXT_PACK.md`

## Batas implementasi saat ini

T-0001 sampai T-0007 menyelesaikan foundation runtime. T-0100 dan T-0101 memulai Phase 1 dengan storage project transaksional, typed desktop commands, system file picker, dan UI lifecycle lengkap. T-0110 menambahkan metadata schema 2 dan persistent job state foundation; T-0111 menambahkan executor background native yang bounded tanpa Tauri/Python/UI integration. Operasi analitik belum diimplementasikan; sidecar Python tetap dependency-free.

MVP berakhir ketika pengguna dapat mengimpor Excel/CSV, melakukan profiling, cleaning, transformasi, join, deteksi duplikasi/outlier/rule, melihat visualisasi, menyimpan workflow, menjalankannya ulang, dan mengekspor hasil beserta audit trail.
