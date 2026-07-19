# Codex Skill Catalog

Skill adalah instruksi workflow reusable. Skill tidak menggantikan AGENTS.md; skill dipanggil untuk jenis pekerjaan tertentu.

## Struktur skill yang disarankan
```text
.codex/skills/<skill-name>/
├── SKILL.md
├── references/
├── scripts/
└── templates/
```

## SK-001 repository-orientation
Trigger: task pertama pada repo atau perubahan lintas modul.
Workflow:
1. Baca AGENTS dan context pack.
2. Petakan package, scripts, tests, dan boundaries.
3. Identifikasi existing pattern yang harus diikuti.
4. Keluarkan impact map sebelum edit.

## SK-002 contract-first-feature
Trigger: command/event/API/operation baru.
Workflow:
1. Tambah schema canonical.
2. Generate types tiga bahasa.
3. Tambah contract tests.
4. Implement engine/Rust/UI secara berurutan.
5. Uji backward compatibility.

## SK-003 analytics-operation-builder
Trigger: transform, statistic, atau detector baru.
Workflow:
1. Tulis definisi metode dan edge cases.
2. Buat golden dataset dan expected result.
3. Implement pure function/core executor.
4. Tambah operation registry dan schema parameter.
5. Tambah explanation/warnings.
6. Benchmark dan memory test.

## SK-004 desktop-ui-feature
Trigger: halaman/komponen UI baru.
Workflow:
1. Gunakan primitive existing.
2. Definisikan loading/empty/error/cancelled.
3. Integrasikan typed query/mutation/event.
4. Tambah keyboard/accessibility test.
5. Tambah screenshot/e2e flow.

## SK-005 workflow-node-builder
Trigger: node workflow baru.
Workflow:
1. Definisikan typed ports.
2. Definisikan parameter JSON schema.
3. Tambah validator dan executor mapping.
4. Tambah icon/label/help text.
5. Tambah graph and rerun tests.

## SK-006 bug-investigation
Trigger: bug, crash, mismatch hasil.
Workflow:
1. Reproduksi dengan fixture minimal.
2. Tentukan layer penyebab.
3. Tulis failing regression test.
4. Perbaiki root cause, bukan symptom.
5. Jalankan affected and cross-layer tests.
6. Catat data-integrity impact.

## SK-007 accuracy-audit
Trigger: sebelum menerima algoritma/transformasi numerik.
Workflow:
1. Bandingkan dengan implementasi independen/reference calculation.
2. Uji null, ties, extreme values, locale, precision.
3. Uji determinism.
4. Dokumentasikan tolerance dan limitation.
5. Tolak bila hanya “kelihatan benar”.

## SK-008 release-hardening
Trigger: milestone/release candidate.
Workflow:
1. Full quality gates.
2. Migration rehearsal and rollback.
3. Installer smoke test.
4. Dependency/license/SBOM scan.
5. Crash recovery and corrupted project test.
6. Release manifest.
