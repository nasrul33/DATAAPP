# Generated from packages/contracts/schemas/engine-handshake-response.schema.json.
# Schema SHA-256: 2b93ccd1ecd92cdc5031ec425d5728bbbd9ee29c97b3909e9081545c82379c5d.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class EngineHandshakeResponse:
    """Engine identity and health returned after a successful startup handshake."""

    capabilities: list[str]
    engine_version: str
    healthy: bool
    protocol_version: str
    python_version: str
    request_id: str
    status: str
