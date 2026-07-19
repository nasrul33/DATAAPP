# Teratai Analytics Desktop

Fondasi monorepo untuk aplikasi desktop analitik data yang offline-first, reproducible, explainable, dan menjaga kepemilikan data lokal.

Versi fondasi: 0.1.0
Target utama: Windows, Tauri 2.x, Python 3.12

## Prasyarat

Toolchain yang telah diverifikasi untuk T-0001 sampai T-0003:

- Node.js 24.17.0 atau kompatibel dengan `>=24.0.0`.
- pnpm 11.9.0.
- Rust stable dengan Cargo dan komponen `rustfmt` serta `clippy` (diverifikasi pada Rust 1.97.1).
- uv 0.11.29 atau lebih baru yang kompatibel.
- Python 3.12; uv akan memilih atau menyiapkan interpreter yang sesuai.

Versi CI dikunci melalui `.node-version`, `.python-version`, `rust-toolchain.toml`, field `packageManager`, dan `tool.uv.required-version`. Perubahan versi harus dilakukan secara eksplisit dan diverifikasi dengan seluruh quality gate.

## Setup lokal

Jalankan dari root repository:

```powershell
pnpm install --frozen-lockfile
cargo check --workspace --locked
uv sync --frozen
```

Perintah tersebut telah diverifikasi pada Windows 11 dengan Python 3.12.6. Lockfile `pnpm-lock.yaml`, `Cargo.lock`, dan `uv.lock` harus ikut disimpan agar resolusi workspace konsisten.

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
apps/desktop              TypeScript desktop boundary; shell Tauri dibuat pada T-0005
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

## Batas fondasi

T-0001 sampai T-0004 tidak mengimplementasikan product feature, React UI, Tauri shell, analytics runtime, IPC runtime, atau persistence. Dependency yang ditambahkan hanya TypeScript, quality tooling, dan serde untuk generated Rust contract serialization; product dependency tetap masuk melalui task pemiliknya.

MVP berakhir ketika pengguna dapat mengimpor Excel/CSV, melakukan profiling, cleaning, transformasi, join, deteksi duplikasi/outlier/rule, melihat visualisasi, menyimpan workflow, menjalankannya ulang, dan mengekspor hasil beserta audit trail.
