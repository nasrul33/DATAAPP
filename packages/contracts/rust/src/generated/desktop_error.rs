// Generated from packages/contracts/schemas/desktop-error.schema.json.
// Schema SHA-256: ea20f5e5761d54e6d9d6027ec13cbc9896808d1473e085b5abd8205e2b892df4.
// Do not edit manually.

/// User-safe typed error returned by native desktop commands.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct DesktopError {
    /// Stable error taxonomy code.
    pub code: String,
    /// UUID v7 joining UI and native diagnostics.
    pub correlation_id: String,
    /// Safe technical category without secret or path leakage.
    pub detail: String,
    /// Actionable field validation messages.
    pub field_errors: Vec<String>,
    /// Localized user-safe summary.
    pub message: String,
    /// Whether retrying unchanged input can be safe.
    pub retriable: bool,
    /// Optional user-safe recovery guidance.
    pub remediation: Option<String>,
}
