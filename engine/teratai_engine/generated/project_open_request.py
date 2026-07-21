# Generated from packages/contracts/schemas/project-open-request.schema.json.
# Schema SHA-256: ae663b2a4b4b3398e9550ca35f945cb5b854e66a10f46a08620b04451cd9dcfe.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ProjectOpenRequest:
    """Native request for opening or validating one user-approved project directory."""

    project_path: str
    request_id: str
