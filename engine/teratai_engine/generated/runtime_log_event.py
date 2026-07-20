# Generated from packages/contracts/schemas/runtime-log-event.schema.json.
# Schema SHA-256: 10c920293561b971e8ee0e8df8bfb452ae512c0aaea25d21e547a2ec07f24fce.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class RuntimeLogEvent:
    """Safe structured runtime log shared by desktop, native host, and Python engine layers."""

    component: str
    correlation_id: str
    event: str
    layer: str
    level: str
    message: str
    sequence: int
    timestamp: str
