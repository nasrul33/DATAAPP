// Generated from packages/contracts/schemas/job-lifecycle-event.schema.json.
// Schema SHA-256: faf0049de1532cfdb546132c164d093e90d3ccd27ad3f871531d0e9e3cb59453.
// Do not edit manually.

use super::job_descriptor::JobDescriptor;

/// Best-effort desktop notification backed by the durable job snapshot and revision.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct JobLifecycleEvent {
    /// Stable lifecycle event name such as job.progress or job.failed.
    pub event_name: String,
    /// Trusted persistent snapshot after the lifecycle mutation.
    pub job: JobDescriptor,
    /// UTC timestamp of the persisted lifecycle mutation.
    pub occurred_at: String,
    /// Desktop job event protocol version.
    pub protocol_version: String,
    /// Positive per-job sequence equal to the persisted snapshot revision.
    pub sequence: i64,
}
