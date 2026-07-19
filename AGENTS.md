# AGENTS.md — Teratai Analytics Desktop

## 1. Mission
Bangun aplikasi desktop analitik data yang mudah digunakan, akurat, explainable, reproducible, aman, dan relevan untuk auditor serta analis non-programmer.

## 2. Instruction precedence
1. Instruksi pengguna pada task aktif.
2. `AGENTS.md` terdekat dengan file yang diedit.
3. Dokumen arsitektur dan kontrak dalam repo.
4. Konvensi umum proyek.

Jangan mengabaikan konflik. Catat konflik pada `docs/CONTEXT_PACK.md` dan ambil opsi paling aman yang tidak merusak data atau kompatibilitas.

## 3. Mandatory reading before changes
- `docs/PRODUCT.md`
- `docs/ARCHITECTURE.md`
- `docs/PRIMITIVES.md`
- `data-contracts/IPC_CONTRACTS.md`
- bagian task terkait pada `docs/IMPLEMENTATION_PLAN.md`

## 4. Non-negotiable rules
- Data sumber tidak boleh dimodifikasi in-place.
- Semua operasi analitik harus memiliki `operation_id`, input fingerprint, parameter tervalidasi, versi algoritma, timestamp, dan output fingerprint.
- Semua proses panjang berjalan sebagai cancellable background job.
- Jangan memuat seluruh dataset besar ke memori UI.
- UI hanya menerima preview terbatasi, statistik agregat, dan paged/virtualized rows.
- Jangan menyebut anomali sebagai fraud tanpa validasi manusia.
- Tidak boleh ada eksekusi arbitrary Python pada MVP.
- Jangan hardcode path, secret, locale, threshold, atau ukuran batch.
- Gunakan typed error envelope; jangan lempar string error mentah ke UI.
- Semua perubahan schema wajib memiliki migration dan rollback note.
- Jangan menambah dependency sebelum memeriksa kebutuhan, lisensi, ukuran bundle, dan alternatif yang sudah ada.

## 5. Architecture boundaries
- `apps/desktop`: UI React dan integrasi Tauri; tidak berisi algoritma analitik.
- `crates/*`: capability native, file system, process control, secure storage, IPC adapter.
- `engine/*`: analitik Python; tidak mengimpor package UI.
- `packages/contracts`: schema kontrak lintas bahasa; sumber kebenaran tipe.
- `packages/ui`: primitive dan komponen presentasional reusable.
- `packages/workflow`: model DAG, validator, dan execution planning yang independen dari UI.
- `tests/golden`: fixture dan expected result untuk pembuktian akurasi.

## 6. Development workflow
1. Baca requirement dan acceptance criteria.
2. Inspeksi implementasi yang sudah ada sebelum membuat pola baru.
3. Tulis atau perbarui test yang gagal terlebih dahulu untuk bug/fitur terukur.
4. Implementasikan perubahan terkecil yang lengkap end-to-end.
5. Jalankan verifikasi yang relevan.
6. Perbarui artifact terkait dan context pack.
7. Laporkan file berubah, keputusan, test, risiko, dan pekerjaan tersisa.

## 7. Required quality gates
Sebelum menyatakan selesai:
- TypeScript strict lulus tanpa `any` baru yang tidak dibenarkan.
- Rust format, clippy, dan test lulus.
- Python lint, type-check, dan test lulus.
- Kontrak IPC backward-compatible atau memiliki versioning/migration.
- Loading, empty, error, cancellation, dan retry state tersedia.
- Tidak ada data loss pada failure/interruption.
- Golden test tersedia untuk operasi numerik atau analitik baru.
- Dokumentasi dan traceability diperbarui.

## 8. Standard verification commands
```bash
pnpm install --frozen-lockfile
pnpm lint
pnpm typecheck
pnpm test
pnpm test:e2e
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
uv sync --frozen
uv run ruff check engine
uv run mypy engine
uv run pytest engine/tests tests/golden
```
Jalankan subset yang relevan selama iterasi, lalu seluruh gate sebelum milestone.

## 9. Commit and task discipline
- Satu task Codex harus terukur dan idealnya berukuran satu perubahan reviewable.
- Jangan mencampur refactor luas dengan fitur kecuali diwajibkan.
- Format commit: Conventional Commits.
- Setiap PR harus memuat: tujuan, scope, risiko, screenshot bila UI, test, migration, dan artifact yang diperbarui.

## 10. Definition of Done
Task selesai hanya jika implementasi, tests, docs, contracts, error states, dan acceptance criteria seluruhnya terpenuhi. “Build berhasil” saja bukan Definition of Done.
