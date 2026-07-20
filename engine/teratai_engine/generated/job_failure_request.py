# Generated from packages/contracts/schemas/job-failure-request.schema.json.
# Schema SHA-256: 47c9cd3cb462cbe2422cdabcdec37d783ef9d2386492299bb326486ae599de50.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class JobFailureRequest:
    """Safe flat failure update for one persistent background job."""

    correlation_id: str
    error_code: str
    error_message: str
    error_retriable: bool
    expected_revision: int
    job_id: str
