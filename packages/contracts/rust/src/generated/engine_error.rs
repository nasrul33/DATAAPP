// Generated from packages/contracts/schemas/engine-error.schema.json.
// Schema SHA-256: 42036f3c7f8fb0a9f90cb30f5865fa3d43f5409f54eccc8d448a2a0327a0bdb7.
// Do not edit manually.

/// Typed user-safe error returned by the Python engine protocol.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EngineError {
    /// Stable machine-readable error taxonomy code.
    pub code: String,
    /// Identifier joining the error to its originating request.
    pub correlation_id: String,
    /// Technical diagnostic without source data or secrets.
    pub detail: String,
    /// Bounded validation messages for invalid request fields.
    pub field_errors: Vec<String>,
    /// User-safe explanation suitable for the desktop UI.
    pub message: String,
    /// Whether retrying the same request can be offered safely.
    pub retriable: bool,
}
