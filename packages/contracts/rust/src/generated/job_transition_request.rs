// Generated from packages/contracts/schemas/job-transition-request.schema.json.
// Schema SHA-256: 55126a4457417bd3838dd8a32fdee844bdd33b27616ac870e845548eafc97d8d.
// Do not edit manually.

/// Safe optimistic-concurrency request for one persistent job state transition.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct JobTransitionRequest {
    /// UUID v7 trace identity for the atomic transition mutation.
    pub correlation_id: String,
    /// Revision required to prevent a lost concurrent update.
    pub expected_revision: i64,
    /// UUID v7 identity of the job to transition.
    pub job_id: String,
}
