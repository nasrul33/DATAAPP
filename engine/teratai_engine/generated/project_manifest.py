# Generated from packages/contracts/schemas/project-manifest.schema.json.
# Schema SHA-256: afdf6fb9906036421d3a461049ae9bb5f0616709af2c9a273975cbd441843db4.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ProjectManifest:
    """Immutable identity and compatibility metadata stored at a Teratai project root."""

    app_version: str
    created_at: str
    metadata_schema_version: int
    name: str
    project_id: str
    schema_version: str
