// Generated from packages/contracts/schemas/job-progress-update-request.schema.json.
// Schema SHA-256: cd8e496d57508a450d28e4c90b354a8828d6525f115c7c6568570c7f9f4eb804.
// Do not edit manually.

/// Safe flat progress update for one persistent background job.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct JobProgressUpdateRequest {
    /// UUID v7 trace identity for the atomic progress mutation.
    pub correlation_id: String,
    /// Completed progress amount after the update.
    pub current: i64,
    /// Revision required to prevent a lost concurrent update.
    pub expected_revision: i64,
    /// UUID v7 identity of the job to update.
    pub job_id: String,
    /// Safe Indonesian progress message without source data.
    pub message: String,
    /// Stable safe execution phase identifier.
    pub phase: String,
    /// Optional known total progress amount.
    pub total: Option<i64>,
    /// Optional safe progress measurement unit.
    pub unit: Option<String>,
}
