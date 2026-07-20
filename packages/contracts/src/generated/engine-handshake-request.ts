// Generated from packages/contracts/schemas/engine-handshake-request.schema.json.
// Schema SHA-256: 211d905f3786c62a024f2ed53d9526652f127ccd43b890be86ad7f2f10f434f1.
// Do not edit manually.

/** Initial host request used to verify the Python engine protocol before accepting work. */
export interface EngineHandshakeRequest {
  /** Lifecycle command name; the handshake value is engine.handshake. */
  readonly command: string;
  /** Semantic version of the native engine host. */
  readonly host_version: string;
  /** Major and minor engine protocol version expected by the host. */
  readonly protocol_version: string;
  /** Caller-generated identifier echoed by the engine for correlation. */
  readonly request_id: string;
  readonly [additionalProperty: string]: unknown;
}
