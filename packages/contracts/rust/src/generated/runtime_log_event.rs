// Generated from packages/contracts/schemas/runtime-log-event.schema.json.
// Schema SHA-256: 10c920293561b971e8ee0e8df8bfb452ae512c0aaea25d21e547a2ec07f24fce.
// Do not edit manually.

/// Safe structured runtime log shared by desktop, native host, and Python engine layers.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct RuntimeLogEvent {
    /// Stable component identifier without machine-specific paths.
    pub component: String,
    /// UUID v7 joining events for one end-to-end operation.
    pub correlation_id: String,
    /// Stable dot-separated event identifier.
    pub event: String,
    /// Runtime boundary that emitted the event: desktop, native, or engine.
    pub layer: String,
    /// Severity name: DEBUG, INFO, WARNING, or ERROR.
    pub level: String,
    /// User-safe diagnostic that excludes source data and secrets.
    pub message: String,
    /// Monotonic sequence within the correlated runtime exchange.
    pub sequence: i64,
    /// ISO-8601 UTC timestamp ending in Z.
    pub timestamp: String,
}
