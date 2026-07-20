// Generated from packages/contracts/schemas/job-progress-update-request.schema.json.
// Schema SHA-256: cd8e496d57508a450d28e4c90b354a8828d6525f115c7c6568570c7f9f4eb804.
// Do not edit manually.

/** Safe flat progress update for one persistent background job. */
export interface JobProgressUpdateRequest {
  /** UUID v7 trace identity for the atomic progress mutation. */
  readonly correlation_id: string;
  /** Completed progress amount after the update. */
  readonly current: number;
  /** Revision required to prevent a lost concurrent update. */
  readonly expected_revision: number;
  /** UUID v7 identity of the job to update. */
  readonly job_id: string;
  /** Safe Indonesian progress message without source data. */
  readonly message: string;
  /** Stable safe execution phase identifier. */
  readonly phase: string;
  /** Optional known total progress amount. */
  readonly total?: number;
  /** Optional safe progress measurement unit. */
  readonly unit?: string;
  readonly [additionalProperty: string]: unknown;
}
