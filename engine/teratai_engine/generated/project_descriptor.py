# Generated from packages/contracts/schemas/project-descriptor.schema.json.
# Schema SHA-256: 7f0e216e6cc84a1210efea34a63192bc9016da8eedb92044854c752023777392.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ProjectDescriptor:
    """Validated project identity returned by the native project core."""

    created_at: str
    metadata_schema_version: int
    name: str
    project_id: str
    project_path: str
    schema_version: str
