// Generated from packages/contracts/schemas/job-transition-request.schema.json.
// Schema SHA-256: 55126a4457417bd3838dd8a32fdee844bdd33b27616ac870e845548eafc97d8d.
// Do not edit manually.

/** Safe optimistic-concurrency request for one persistent job state transition. */
export interface JobTransitionRequest {
  /** UUID v7 trace identity for the atomic transition mutation. */
  readonly correlation_id: string;
  /** Revision required to prevent a lost concurrent update. */
  readonly expected_revision: number;
  /** UUID v7 identity of the job to transition. */
  readonly job_id: string;
  readonly [additionalProperty: string]: unknown;
}
