// Generated from packages/contracts/schemas/correlation-request.schema.json.
// Schema SHA-256: 165eceab4eb5c64ba7c852b4b936d671ca3186d0c3e53d067a18ed669eb9e535.
// Do not edit manually.

/** Minimal request used by native commands that only require correlation. */
export interface CorrelationRequest {
  /** Lowercase UUID v7 correlation identifier. */
  readonly request_id: string;
  readonly [additionalProperty: string]: unknown;
}
