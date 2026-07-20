# Rollback note — metadata schema 2

Schema `0002_job_runtime` tidak memiliki downgrade in-place yang aman setelah migration berhasil. Aplikasi lama wajib menolak `PRAGMA user_version = 2`; aplikasi tidak boleh menghapus tabel `job`, `job_event`, atau riwayat `audit_event` untuk memaksakan kompatibilitas.

Jika migration terinterupsi sebelum berhasil, recovery wajib memulihkan `metadata.sqlite` dan manifest dari backup schema 1 yang sudah divalidasi, lalu membuktikan kembali identitas project, integritas SQLite, versi schema, dan rantai audit sebelum recovery marker dihapus. Jika pemulihan tidak dapat dibuktikan lengkap, marker dan backup dipertahankan agar project tetap berstatus recovery-required dan tidak dibuka secara normal.

Project pengguna harus dipertahankan utuh dan dibuka kembali menggunakan binary yang mendukung schema 2. Penghapusan project atau riwayat job/audit bukan prosedur rollback.
