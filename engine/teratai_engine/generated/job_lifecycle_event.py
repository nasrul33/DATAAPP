# Generated from packages/contracts/schemas/job-lifecycle-event.schema.json.
# Schema SHA-256: faf0049de1532cfdb546132c164d093e90d3ccd27ad3f871531d0e9e3cb59453.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass

from .job_descriptor import JobDescriptor


@dataclass(frozen=True, slots=True)
class JobLifecycleEvent:
    """Best-effort desktop notification backed by the durable job snapshot and revision."""

    event_name: str
    job: JobDescriptor
    occurred_at: str
    protocol_version: str
    sequence: int
