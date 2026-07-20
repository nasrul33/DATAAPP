// Generated from packages/contracts/schemas/project-manifest.schema.json.
// Schema SHA-256: afdf6fb9906036421d3a461049ae9bb5f0616709af2c9a273975cbd441843db4.
// Do not edit manually.

/// Immutable identity and compatibility metadata stored at a Teratai project root.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProjectManifest {
    /// Teratai version that created the project.
    pub app_version: String,
    /// ISO-8601 UTC creation timestamp.
    pub created_at: String,
    /// Required `SQLite` metadata schema version.
    pub metadata_schema_version: i64,
    /// Human-readable project name.
    pub name: String,
    /// UUID v7 immutable project identity.
    pub project_id: String,
    /// Project manifest semantic version.
    pub schema_version: String,
}
