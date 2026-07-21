// Generated from packages/contracts/schemas/desktop-error.schema.json.
// Schema SHA-256: ea20f5e5761d54e6d9d6027ec13cbc9896808d1473e085b5abd8205e2b892df4.
// Do not edit manually.

/** User-safe typed error returned by native desktop commands. */
export interface DesktopError {
  /** Stable error taxonomy code. */
  readonly code: string;
  /** UUID v7 joining UI and native diagnostics. */
  readonly correlation_id: string;
  /** Safe technical category without secret or path leakage. */
  readonly detail: string;
  /** Actionable field validation messages. */
  readonly field_errors: readonly string[];
  /** Localized user-safe summary. */
  readonly message: string;
  /** Whether retrying unchanged input can be safe. */
  readonly retriable: boolean;
  /** Optional user-safe recovery guidance. */
  readonly remediation?: string;
  readonly [additionalProperty: string]: unknown;
}
