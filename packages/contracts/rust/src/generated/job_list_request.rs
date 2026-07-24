// Generated from packages/contracts/schemas/job-list-request.schema.json.
// Schema SHA-256: ab8a8451296d2a6f3c5becfa5b9ef209db56f9f510ece6c49fa7546df3c1ffb6.
// Do not edit manually.

/// Typed bounded keyset request for project-scoped persistent jobs.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct JobListRequest {
    /// Lowercase UUID v7 identity for this desktop request.
    pub correlation_id: String,
    /// Requested page size validated natively within one through one hundred.
    pub limit: i64,
    /// Optional lowercase UUID v7 from the previous page cursor.
    pub cursor_job_id: Option<String>,
    /// Optional UTC timestamp from the previous page cursor.
    pub cursor_updated_at: Option<String>,
}
