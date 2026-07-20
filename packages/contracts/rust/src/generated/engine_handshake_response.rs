// Generated from packages/contracts/schemas/engine-handshake-response.schema.json.
// Schema SHA-256: 2b93ccd1ecd92cdc5031ec425d5728bbbd9ee29c97b3909e9081545c82379c5d.
// Do not edit manually.

/// Engine identity and health returned after a successful startup handshake.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EngineHandshakeResponse {
    /// Stable capability identifiers available in this engine process.
    pub capabilities: Vec<String>,
    /// Semantic version of the Python analytics engine.
    pub engine_version: String,
    /// True only after all startup health checks complete.
    pub healthy: bool,
    /// Major and minor protocol version implemented by the engine.
    pub protocol_version: String,
    /// Major, minor, and patch version of the active Python runtime.
    pub python_version: String,
    /// Request identifier copied from the handshake request.
    pub request_id: String,
    /// User-safe lifecycle status; a healthy engine reports ready.
    pub status: String,
}
