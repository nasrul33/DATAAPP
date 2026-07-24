# Generated from packages/contracts/schemas/job-list-request.schema.json.
# Schema SHA-256: ab8a8451296d2a6f3c5becfa5b9ef209db56f9f510ece6c49fa7546df3c1ffb6.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class JobListRequest:
    """Typed bounded keyset request for project-scoped persistent jobs."""

    correlation_id: str
    limit: int
    cursor_job_id: str | None = None
    cursor_updated_at: str | None = None
