// Generated from packages/contracts/schemas/job-failure-request.schema.json.
// Schema SHA-256: 47c9cd3cb462cbe2422cdabcdec37d783ef9d2386492299bb326486ae599de50.
// Do not edit manually.

/// Safe flat failure update for one persistent background job.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct JobFailureRequest {
    /// UUID v7 trace identity for the atomic failure mutation.
    pub correlation_id: String,
    /// Stable safe failure code.
    pub error_code: String,
    /// Safe actionable failure message without raw errors.
    pub error_message: String,
    /// Whether retrying the failed job is safe and permitted.
    pub error_retriable: bool,
    /// Revision required to prevent a lost concurrent update.
    pub expected_revision: i64,
    /// UUID v7 identity of the job to fail.
    pub job_id: String,
}
