// Generated from packages/contracts/schemas/job-failure-request.schema.json.
// Schema SHA-256: 47c9cd3cb462cbe2422cdabcdec37d783ef9d2386492299bb326486ae599de50.
// Do not edit manually.

/** Safe flat failure update for one persistent background job. */
export interface JobFailureRequest {
  /** UUID v7 trace identity for the atomic failure mutation. */
  readonly correlation_id: string;
  /** Stable safe failure code. */
  readonly error_code: string;
  /** Safe actionable failure message without raw errors. */
  readonly error_message: string;
  /** Whether retrying the failed job is safe and permitted. */
  readonly error_retriable: boolean;
  /** Revision required to prevent a lost concurrent update. */
  readonly expected_revision: number;
  /** UUID v7 identity of the job to fail. */
  readonly job_id: string;
  readonly [additionalProperty: string]: unknown;
}
