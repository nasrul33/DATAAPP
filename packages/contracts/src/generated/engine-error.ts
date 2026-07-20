// Generated from packages/contracts/schemas/engine-error.schema.json.
// Schema SHA-256: 42036f3c7f8fb0a9f90cb30f5865fa3d43f5409f54eccc8d448a2a0327a0bdb7.
// Do not edit manually.

/** Typed user-safe error returned by the Python engine protocol. */
export interface EngineError {
  /** Stable machine-readable error taxonomy code. */
  readonly code: string;
  /** Identifier joining the error to its originating request. */
  readonly correlation_id: string;
  /** Technical diagnostic without source data or secrets. */
  readonly detail: string;
  /** Bounded validation messages for invalid request fields. */
  readonly field_errors: readonly string[];
  /** User-safe explanation suitable for the desktop UI. */
  readonly message: string;
  /** Whether retrying the same request can be offered safely. */
  readonly retriable: boolean;
  readonly [additionalProperty: string]: unknown;
}
