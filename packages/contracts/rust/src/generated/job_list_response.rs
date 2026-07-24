// Generated from packages/contracts/schemas/job-list-response.schema.json.
// Schema SHA-256: 8d85ee15c87acb87cf5861195c51194307708b77ca4640025d50ac1f4ffa1698.
// Do not edit manually.

use super::job_descriptor::JobDescriptor;

/// Bounded project-scoped job page with an optional opaque keyset cursor pair.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct JobListResponse {
    /// Trusted persistent job snapshots ordered newest first.
    pub items: Vec<JobDescriptor>,
    /// Lowercase UUID v7 for the next keyset page.
    pub next_cursor_job_id: Option<String>,
    /// UTC timestamp for the next keyset page.
    pub next_cursor_updated_at: Option<String>,
}
