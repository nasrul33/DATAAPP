# Generated from packages/contracts/schemas/job-transition-request.schema.json.
# Schema SHA-256: 55126a4457417bd3838dd8a32fdee844bdd33b27616ac870e845548eafc97d8d.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class JobTransitionRequest:
    """Safe optimistic-concurrency request for one persistent job state transition."""

    correlation_id: str
    expected_revision: int
    job_id: str
