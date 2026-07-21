// Generated from packages/contracts/schemas/job-enqueue-request.schema.json.
// Schema SHA-256: 2e6075b68c0b6b640f3661c65aee65d4db3b7a244a1506fde3c00244bfeccd0c.
// Do not edit manually.

/** Safe flat request to persist a newly queued background job. */
export interface JobEnqueueRequest {
  /** UUID v7 trace identity for the atomic enqueue mutation. */
  readonly correlation_id: string;
  /** UUID v7 identity assigned before the enqueue mutation. */
  readonly job_id: string;
  /** Stable job kind identifier. */
  readonly kind: string;
  /** Optional known total progress amount. */
  readonly progress_total?: number;
  /** Optional safe progress measurement unit. */
  readonly progress_unit?: string;
  readonly [additionalProperty: string]: unknown;
}
