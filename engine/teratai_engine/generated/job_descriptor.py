# Generated from packages/contracts/schemas/job-descriptor.schema.json.
# Schema SHA-256: 0ba7668ff3404962867d0a8ed0cf915f0e8d9ed74d1d559495e9d90a3622c89e.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class JobDescriptor:
    """Persistent, safe summary of a background job without source rows or local paths."""

    correlation_id: str
    created_at: str
    job_id: str
    kind: str
    progress_current: int
    project_id: str
    revision: int
    status: str
    updated_at: str
    error_code: str | None = None
    error_message: str | None = None
    error_retriable: bool | None = None
    finished_at: str | None = None
    progress_message: str | None = None
    progress_phase: str | None = None
    progress_total: int | None = None
    progress_unit: str | None = None
    started_at: str | None = None
