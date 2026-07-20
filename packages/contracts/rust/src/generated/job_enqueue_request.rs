// Generated from packages/contracts/schemas/job-enqueue-request.schema.json.
// Schema SHA-256: 2e6075b68c0b6b640f3661c65aee65d4db3b7a244a1506fde3c00244bfeccd0c.
// Do not edit manually.

/// Safe flat request to persist a newly queued background job.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct JobEnqueueRequest {
    /// UUID v7 trace identity for the atomic enqueue mutation.
    pub correlation_id: String,
    /// UUID v7 identity assigned before the enqueue mutation.
    pub job_id: String,
    /// Stable job kind identifier.
    pub kind: String,
    /// Optional known total progress amount.
    pub progress_total: Option<i64>,
    /// Optional safe progress measurement unit.
    pub progress_unit: Option<String>,
}
