# Generated from packages/contracts/schemas/job-enqueue-request.schema.json.
# Schema SHA-256: 2e6075b68c0b6b640f3661c65aee65d4db3b7a244a1506fde3c00244bfeccd0c.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class JobEnqueueRequest:
    """Safe flat request to persist a newly queued background job."""

    correlation_id: str
    job_id: str
    kind: str
    progress_total: int | None = None
    progress_unit: str | None = None
