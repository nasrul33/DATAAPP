// Generated from packages/contracts/schemas/contract-metadata.schema.json.
// Schema SHA-256: db8b3860cc8a21ef8174d42662658f12d4626b608156c4b2494470025c748056.
// Do not edit manually.

/// Version metadata embedded in generated Teratai cross-language contracts.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ContractMetadata {
    /// Monotonic revision of the generator contract subset.
    pub generator_revision: i64,
    /// Major and minor engine protocol version.
    pub protocol_version: String,
    /// Stable kebab-case canonical schema name.
    pub schema_name: String,
    /// Semantic version of this schema.
    pub schema_version: String,
}
