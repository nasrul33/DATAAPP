# Generated from packages/contracts/schemas/desktop-error.schema.json.
# Schema SHA-256: ea20f5e5761d54e6d9d6027ec13cbc9896808d1473e085b5abd8205e2b892df4.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class DesktopError:
    """User-safe typed error returned by native desktop commands."""

    code: str
    correlation_id: str
    detail: str
    field_errors: list[str]
    message: str
    retriable: bool
    remediation: str | None = None
