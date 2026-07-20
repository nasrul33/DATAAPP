PRAGMA foreign_keys = ON;

CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version > 0),
    name TEXT NOT NULL UNIQUE CHECK (length(trim(name)) > 0),
    applied_at TEXT NOT NULL CHECK (applied_at GLOB '????-??-??T*Z')
) STRICT;

CREATE TABLE project (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    project_id TEXT NOT NULL UNIQUE CHECK (length(project_id) = 36),
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 120),
    created_at TEXT NOT NULL CHECK (created_at GLOB '????-??-??T*Z'),
    app_version TEXT NOT NULL CHECK (length(trim(app_version)) > 0),
    manifest_schema_version TEXT NOT NULL CHECK (length(trim(manifest_schema_version)) > 0)
) STRICT;

CREATE TABLE audit_event (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE CHECK (length(event_id) = 36),
    actor TEXT NOT NULL CHECK (length(trim(actor)) > 0),
    action TEXT NOT NULL CHECK (length(trim(action)) > 0),
    target_type TEXT NOT NULL CHECK (length(trim(target_type)) > 0),
    target_id TEXT NOT NULL CHECK (length(trim(target_id)) > 0),
    before_hash TEXT,
    after_hash TEXT,
    occurred_at TEXT NOT NULL CHECK (occurred_at GLOB '????-??-??T*Z'),
    correlation_id TEXT NOT NULL CHECK (length(correlation_id) = 36)
) STRICT;

CREATE TRIGGER audit_event_prevent_update
BEFORE UPDATE ON audit_event
BEGIN
    SELECT RAISE(ABORT, 'audit_event is append-only');
END;

CREATE TRIGGER audit_event_prevent_delete
BEFORE DELETE ON audit_event
BEGIN
    SELECT RAISE(ABORT, 'audit_event is append-only');
END;

CREATE INDEX audit_event_correlation_sequence_idx
ON audit_event (correlation_id, sequence);

CREATE INDEX audit_event_target_sequence_idx
ON audit_event (target_type, target_id, sequence);

PRAGMA user_version = 1;
