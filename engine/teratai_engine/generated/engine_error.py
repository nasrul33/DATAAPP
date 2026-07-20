# Generated from packages/contracts/schemas/engine-error.schema.json.
# Schema SHA-256: 42036f3c7f8fb0a9f90cb30f5865fa3d43f5409f54eccc8d448a2a0327a0bdb7.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class EngineError:
    """Typed user-safe error returned by the Python engine protocol."""

    code: str
    correlation_id: str
    detail: str
    field_errors: list[str]
    message: str
    retriable: bool
