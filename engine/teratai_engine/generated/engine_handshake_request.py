# Generated from packages/contracts/schemas/engine-handshake-request.schema.json.
# Schema SHA-256: 211d905f3786c62a024f2ed53d9526652f127ccd43b890be86ad7f2f10f434f1.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class EngineHandshakeRequest:
    """Initial host request used to verify the Python engine protocol before accepting work."""

    command: str
    host_version: str
    protocol_version: str
    request_id: str
