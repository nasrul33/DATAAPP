# Generated from packages/contracts/schemas/correlation-request.schema.json.
# Schema SHA-256: 165eceab4eb5c64ba7c852b4b936d671ca3186d0c3e53d067a18ed669eb9e535.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class CorrelationRequest:
    """Minimal request used by native commands that only require correlation."""

    request_id: str
