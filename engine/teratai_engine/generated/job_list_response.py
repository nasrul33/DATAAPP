# Generated from packages/contracts/schemas/job-list-response.schema.json.
# Schema SHA-256: 8d85ee15c87acb87cf5861195c51194307708b77ca4640025d50ac1f4ffa1698.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass

from .job_descriptor import JobDescriptor


@dataclass(frozen=True, slots=True)
class JobListResponse:
    """Bounded project-scoped job page with an optional opaque keyset cursor pair."""

    items: list[JobDescriptor]
    next_cursor_job_id: str | None = None
    next_cursor_updated_at: str | None = None
