# Rollback note — metadata schema 1

Schema `0001_project_core` adalah baseline untuk project baru dan tidak memiliki downgrade in-place. Rollback aplikasi harus menolak membuka project dengan `PRAGMA user_version = 1` bila binary tidak mendukungnya; jangan menghapus tabel atau audit event.

Untuk project yang belum pernah dipakai dan dibuat selama pengujian, rollback dilakukan dengan menghapus seluruh direktori project melalui workflow cleanup pengujian. Project pengguna harus dipertahankan utuh dan dipulihkan menggunakan binary yang mendukung schema version 1.
