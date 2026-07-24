// Generated from packages/contracts/schemas/job-list-request.schema.json.
// Schema SHA-256: ab8a8451296d2a6f3c5becfa5b9ef209db56f9f510ece6c49fa7546df3c1ffb6.
// Do not edit manually.

/** Typed bounded keyset request for project-scoped persistent jobs. */
export interface JobListRequest {
  /** Lowercase UUID v7 identity for this desktop request. */
  readonly correlation_id: string;
  /** Requested page size validated natively within one through one hundred. */
  readonly limit: number;
  /** Optional lowercase UUID v7 from the previous page cursor. */
  readonly cursor_job_id?: string;
  /** Optional UTC timestamp from the previous page cursor. */
  readonly cursor_updated_at?: string;
  readonly [additionalProperty: string]: unknown;
}
