// Generated from packages/contracts/schemas/job-descriptor.schema.json.
// Schema SHA-256: 0ba7668ff3404962867d0a8ed0cf915f0e8d9ed74d1d559495e9d90a3622c89e.
// Do not edit manually.

/// Persistent, safe summary of a background job without source rows or local paths.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct JobDescriptor {
    /// UUID v7 trace identity for the job mutation.
    pub correlation_id: String,
    /// ISO-8601 UTC timestamp when the job was created.
    pub created_at: String,
    /// UUID v7 immutable job identity.
    pub job_id: String,
    /// Stable job kind identifier.
    pub kind: String,
    /// Completed progress amount.
    pub progress_current: i64,
    /// UUID v7 project identity that owns the job.
    pub project_id: String,
    /// Positive optimistic-concurrency revision.
    pub revision: i64,
    /// Current lifecycle status such as QUEUED or RUNNING.
    pub status: String,
    /// ISO-8601 UTC timestamp of the latest job change.
    pub updated_at: String,
    /// Stable safe failure code.
    pub error_code: Option<String>,
    /// Safe actionable failure message without raw errors.
    pub error_message: Option<String>,
    /// Whether retrying the job is safe and permitted.
    pub error_retriable: Option<bool>,
    /// ISO-8601 UTC timestamp when execution finished.
    pub finished_at: Option<String>,
    /// Safe Indonesian progress message without source data.
    pub progress_message: Option<String>,
    /// Stable safe execution phase identifier.
    pub progress_phase: Option<String>,
    /// Known total progress amount.
    pub progress_total: Option<i64>,
    /// Safe progress measurement unit.
    pub progress_unit: Option<String>,
    /// ISO-8601 UTC timestamp when execution started.
    pub started_at: Option<String>,
}
