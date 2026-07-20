// Generated from packages/contracts/schemas/project-create-request.schema.json.
// Schema SHA-256: a933ec6c79528f5e9a87ce7e8dc72e0e9202ec6189e1a4ea63db34f203a055d9.
// Do not edit manually.

/// Validated native request for creating one local project directory.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProjectCreateRequest {
    /// Human-readable project name.
    pub name: String,
    /// UUID v7 identity assigned to the new project.
    pub project_id: String,
    /// Absolute user-approved target directory ending in .teratai.
    pub project_path: String,
    /// UUID v7 correlation identifier.
    pub request_id: String,
}
