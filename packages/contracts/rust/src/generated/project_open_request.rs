// Generated from packages/contracts/schemas/project-open-request.schema.json.
// Schema SHA-256: ae663b2a4b4b3398e9550ca35f945cb5b854e66a10f46a08620b04451cd9dcfe.
// Do not edit manually.

/// Native request for opening or validating one user-approved project directory.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProjectOpenRequest {
    /// Absolute user-approved `.teratai` project directory.
    pub project_path: String,
    /// Lowercase UUID v7 correlation identifier.
    pub request_id: String,
}
