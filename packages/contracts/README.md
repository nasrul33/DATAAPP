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
