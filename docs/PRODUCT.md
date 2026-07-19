# Product Definition

## Visi
Teratai Analytics Desktop membantu auditor dan analis memperoleh insight yang dapat dibuktikan dari data tanpa ketergantungan pada coding.

## Pengguna utama
- Auditor/evaluator: analisis risiko, anomali, sampling, temuan.
- Data analyst: preparation, exploration, statistics, modeling dasar.
- Reviewer: menilai metodologi, bukti, dan kesimpulan.
- Operator data: impor, validasi, dan standardisasi data.

## Jobs to be done
1. Membuka data dari Excel/CSV secara aman.
2. Memahami kualitas data dalam menit, bukan jam.
3. Membersihkan dan menggabungkan data tanpa merusak sumber.
4. Menjalankan analisis yang dapat diulang dan diperiksa.
5. Menjelaskan mengapa suatu baris ditandai berisiko.
6. Mengubah indikasi menjadi temuan dengan bukti.
7. Mengekspor hasil yang konsisten untuk kertas kerja atau laporan.

## Scope MVP
- Project/workspace lokal.
- Import Excel, CSV, Parquet.
- Schema inference dan override.
- Profiling dan quality report.
- Cleaning dan transformation dasar.
- Join, union, group, pivot/unpivot.
- Duplicate, near-duplicate, IQR, modified Z-score, Benford, rules.
- Risk scoring explainable.
- Data table virtualized dan charts utama.
- Workflow DAG tersimpan dan rerunnable.
- Findings register dan evidence reference dasar.
- Export CSV, XLSX, Parquet, PNG, JSON manifest.
- Immutable audit event log.

## Out of scope MVP
- Cloud collaboration dan real-time multiuser.
- Arbitrary user Python/R execution.
- Auto-claim fraud.
- Deep learning dan generative AI terhadap data sensitif.
- Direct write-back ke database sumber.
- Plugin marketplace publik.

## North-star metrics
- Time-to-first-profile ≤ 60 detik untuk 100 ribu baris pada perangkat target.
- 100% operasi MVP menghasilkan lineage dan reproducibility manifest.
- Hasil agregasi deterministik cocok dengan golden expected values.
- Crash recovery tidak merusak project.
- Pengguna non-programmer menyelesaikan workflow standar tanpa dokumentasi eksternal.

## UX principles
- Progressive disclosure: fungsi umum terlihat, parameter lanjutan tidak mengganggu.
- Preview before execute: tampilkan dampak dan estimasi sebelum operasi destruktif/tambahan besar.
- Explainability by default: tampilkan alasan, parameter, dan populasi pembanding.
- Clear status: queued, running, completed, failed, cancelled.
- Audit language: “indikasi”, “perlu reviu”, “dikonfirmasi”, bukan tuduhan otomatis.
