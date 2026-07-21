# Generated from packages/contracts/schemas/project-create-request.schema.json.
# Schema SHA-256: a933ec6c79528f5e9a87ce7e8dc72e0e9202ec6189e1a4ea63db34f203a055d9.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ProjectCreateRequest:
    """Validated native request for creating one local project directory."""

    name: str
    project_id: str
    project_path: str
    request_id: str
