# Generated from packages/contracts/schemas/contract-metadata.schema.json.
# Schema SHA-256: db8b3860cc8a21ef8174d42662658f12d4626b608156c4b2494470025c748056.
# Do not edit manually.
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ContractMetadata:
    """Version metadata embedded in generated Teratai cross-language contracts."""

    generator_revision: int
    protocol_version: str
    schema_name: str
    schema_version: str
