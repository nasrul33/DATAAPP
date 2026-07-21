// Generated from packages/contracts/schemas/project-descriptor.schema.json.
// Schema SHA-256: 7f0e216e6cc84a1210efea34a63192bc9016da8eedb92044854c752023777392.
// Do not edit manually.

/// Validated project identity returned by the native project core.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProjectDescriptor {
    /// ISO-8601 UTC creation timestamp.
    pub created_at: String,
    /// Validated `SQLite` metadata schema version.
    pub metadata_schema_version: i64,
    /// Human-readable project name.
    pub name: String,
    /// UUID v7 immutable project identity.
    pub project_id: String,
    /// Canonical absolute project directory.
    pub project_path: String,
    /// Validated project manifest semantic version.
    pub schema_version: String,
}
