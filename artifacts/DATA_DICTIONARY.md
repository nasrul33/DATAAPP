# Data Dictionary — Metadata SQLite

Schema aktif: `1`
Migration: `migrations/metadata-sqlite/0001_project_core.sql`

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
