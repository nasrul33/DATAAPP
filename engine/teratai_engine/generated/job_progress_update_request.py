# Generated from packages/contracts/schemas/job-progress-update-request.schema.json.
# Schema SHA-256: cd8e496d57508a450d28e4c90b354a8828d6525f115c7c6568570c7f9f4eb804.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class JobProgressUpdateRequest:
    """Safe flat progress update for one persistent background job."""

    correlation_id: str
    current: int
    expected_revision: int
    job_id: str
    message: str
    phase: str
    total: int | None = None
    unit: str | None = None
