# Generated from packages/contracts/schemas/job-get-request.schema.json.
# Schema SHA-256: 1b708a1b590439a9b73195ef94cb9ab049100e2d0c2352c90815c96b3acf9680.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class JobGetRequest:
    """Typed desktop request for one project-scoped persistent job snapshot."""

    correlation_id: str
    job_id: str
