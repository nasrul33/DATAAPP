// Generated from packages/contracts/schemas/engine-handshake-response.schema.json.
// Schema SHA-256: 2b93ccd1ecd92cdc5031ec425d5728bbbd9ee29c97b3909e9081545c82379c5d.
// Do not edit manually.

/** Engine identity and health returned after a successful startup handshake. */
export interface EngineHandshakeResponse {
  /** Stable capability identifiers available in this engine process. */
  readonly capabilities: readonly string[];
  /** Semantic version of the Python analytics engine. */
  readonly engine_version: string;
  /** True only after all startup health checks complete. */
  readonly healthy: boolean;
  /** Major and minor protocol version implemented by the engine. */
  readonly protocol_version: string;
  /** Major, minor, and patch version of the active Python runtime. */
  readonly python_version: string;
  /** Request identifier copied from the handshake request. */
  readonly request_id: string;
  /** User-safe lifecycle status; a healthy engine reports ready. */
  readonly status: string;
  readonly [additionalProperty: string]: unknown;
}
