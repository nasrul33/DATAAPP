CREATE TABLE job (
    job_id TEXT PRIMARY KEY
        CHECK (
            length(job_id) = 36
            AND job_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND job_id NOT GLOB '*[^0-9a-f-]*'
        ),
    project_id TEXT NOT NULL
        CHECK (
            length(project_id) = 36
            AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND project_id NOT GLOB '*[^0-9a-f-]*'
        )
        REFERENCES project (project_id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    kind TEXT NOT NULL
        CHECK (
            length(kind) BETWEEN 1 AND 120
            AND kind GLOB '[a-z]*'
            AND kind NOT GLOB '*[^a-z0-9._-]*'
        ),
    status TEXT NOT NULL
        CHECK (status IN ('QUEUED', 'RUNNING', 'SUCCEEDED', 'FAILED', 'CANCELLING', 'CANCELLED')),
    correlation_id TEXT NOT NULL
        CHECK (
            length(correlation_id) = 36
            AND correlation_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND correlation_id NOT GLOB '*[^0-9a-f-]*'
        ),
    revision INTEGER NOT NULL CHECK (revision > 0),
    created_at TEXT NOT NULL
        CHECK (length(created_at) BETWEEN 20 AND 35 AND created_at GLOB '????-??-??T*Z'),
    started_at TEXT
        CHECK (
            started_at IS NULL
            OR (length(started_at) BETWEEN 20 AND 35 AND started_at GLOB '????-??-??T*Z')
        ),
    finished_at TEXT
        CHECK (
            finished_at IS NULL
            OR (length(finished_at) BETWEEN 20 AND 35 AND finished_at GLOB '????-??-??T*Z')
        ),
    updated_at TEXT NOT NULL
        CHECK (length(updated_at) BETWEEN 20 AND 35 AND updated_at GLOB '????-??-??T*Z'),
    progress_current INTEGER NOT NULL DEFAULT 0 CHECK (progress_current >= 0),
    progress_total INTEGER CHECK (progress_total IS NULL OR progress_total > 0),
    progress_unit TEXT
        CHECK (
            progress_unit IS NULL
            OR (
                length(progress_unit) BETWEEN 1 AND 32
                AND progress_unit = trim(progress_unit)
            )
        ),
    progress_phase TEXT
        CHECK (
            progress_phase IS NULL
            OR (
                length(progress_phase) BETWEEN 1 AND 120
                AND progress_phase GLOB '[a-z]*'
                AND progress_phase NOT GLOB '*[^a-z0-9._-]*'
            )
        ),
    progress_message TEXT
        CHECK (
            progress_message IS NULL
            OR (
                length(progress_message) BETWEEN 1 AND 500
                AND progress_message = trim(progress_message)
            )
        ),
    error_code TEXT
        CHECK (
            error_code IS NULL
            OR (
                length(error_code) BETWEEN 1 AND 120
                AND error_code GLOB '[A-Z]*'
                AND error_code NOT GLOB '*[^A-Z0-9_]*'
            )
        ),
    error_message TEXT
        CHECK (
            error_message IS NULL
            OR (
                length(error_message) BETWEEN 1 AND 500
                AND error_message = trim(error_message)
            )
        ),
    error_retriable INTEGER CHECK (error_retriable IS NULL OR error_retriable IN (0, 1)),
    CHECK (progress_total IS NULL OR progress_current <= progress_total),
    CHECK (
        (
            status IN ('SUCCEEDED', 'FAILED', 'CANCELLED')
            AND finished_at IS NOT NULL
        )
        OR (
            status NOT IN ('SUCCEEDED', 'FAILED', 'CANCELLED')
            AND finished_at IS NULL
        )
    ),
    CHECK (
        (
            status = 'FAILED'
            AND error_code IS NOT NULL
            AND error_message IS NOT NULL
            AND error_retriable IS NOT NULL
        )
        OR (
            status <> 'FAILED'
            AND error_code IS NULL
            AND error_message IS NULL
            AND error_retriable IS NULL
        )
    )
) STRICT;

CREATE TABLE job_event (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE
        CHECK (
            length(event_id) = 36
            AND event_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND event_id NOT GLOB '*[^0-9a-f-]*'
        ),
    job_id TEXT NOT NULL
        CHECK (
            length(job_id) = 36
            AND job_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND job_id NOT GLOB '*[^0-9a-f-]*'
        )
        REFERENCES job (job_id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    event_type TEXT NOT NULL
        CHECK (
            length(event_type) BETWEEN 1 AND 120
            AND event_type GLOB '[a-z]*'
            AND event_type NOT GLOB '*[^a-z0-9._-]*'
        ),
    from_status TEXT
        CHECK (
            from_status IS NULL
            OR from_status IN ('QUEUED', 'RUNNING', 'SUCCEEDED', 'FAILED', 'CANCELLING', 'CANCELLED')
        ),
    to_status TEXT NOT NULL
        CHECK (to_status IN ('QUEUED', 'RUNNING', 'SUCCEEDED', 'FAILED', 'CANCELLING', 'CANCELLED')),
    revision INTEGER NOT NULL CHECK (revision > 0),
    progress_current INTEGER NOT NULL CHECK (progress_current >= 0),
    progress_total INTEGER CHECK (progress_total IS NULL OR progress_total > 0),
    progress_unit TEXT
        CHECK (
            progress_unit IS NULL
            OR (
                length(progress_unit) BETWEEN 1 AND 32
                AND progress_unit = trim(progress_unit)
            )
        ),
    progress_phase TEXT
        CHECK (
            progress_phase IS NULL
            OR (
                length(progress_phase) BETWEEN 1 AND 120
                AND progress_phase GLOB '[a-z]*'
                AND progress_phase NOT GLOB '*[^a-z0-9._-]*'
            )
        ),
    progress_message TEXT
        CHECK (
            progress_message IS NULL
            OR (
                length(progress_message) BETWEEN 1 AND 500
                AND progress_message = trim(progress_message)
            )
        ),
    error_code TEXT
        CHECK (
            error_code IS NULL
            OR (
                length(error_code) BETWEEN 1 AND 120
                AND error_code GLOB '[A-Z]*'
                AND error_code NOT GLOB '*[^A-Z0-9_]*'
            )
        ),
    error_message TEXT
        CHECK (
            error_message IS NULL
            OR (
                length(error_message) BETWEEN 1 AND 500
                AND error_message = trim(error_message)
            )
        ),
    error_retriable INTEGER CHECK (error_retriable IS NULL OR error_retriable IN (0, 1)),
    occurred_at TEXT NOT NULL
        CHECK (length(occurred_at) BETWEEN 20 AND 35 AND occurred_at GLOB '????-??-??T*Z'),
    correlation_id TEXT NOT NULL
        CHECK (
            length(correlation_id) = 36
            AND correlation_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND correlation_id NOT GLOB '*[^0-9a-f-]*'
        ),
    CHECK (progress_total IS NULL OR progress_current <= progress_total),
    CHECK (
        (
            to_status = 'FAILED'
            AND error_code IS NOT NULL
            AND error_message IS NOT NULL
            AND error_retriable IS NOT NULL
        )
        OR (
            to_status <> 'FAILED'
            AND error_code IS NULL
            AND error_message IS NULL
            AND error_retriable IS NULL
        )
    )
) STRICT;

CREATE TRIGGER job_event_prevent_update
BEFORE UPDATE ON job_event
BEGIN
    SELECT RAISE(ABORT, 'job_event is append-only');
END;

CREATE TRIGGER job_event_prevent_delete
BEFORE DELETE ON job_event
BEGIN
    SELECT RAISE(ABORT, 'job_event is append-only');
END;

CREATE INDEX job_status_updated_idx
ON job (status, updated_at DESC, job_id DESC);

CREATE INDEX job_correlation_idx
ON job (correlation_id, updated_at DESC);

CREATE INDEX job_event_job_sequence_idx
ON job_event (job_id, sequence);

CREATE INDEX job_event_correlation_sequence_idx
ON job_event (correlation_id, sequence);

PRAGMA user_version = 2;
