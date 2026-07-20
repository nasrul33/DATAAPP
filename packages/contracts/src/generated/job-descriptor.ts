// Generated from packages/contracts/schemas/job-descriptor.schema.json.
// Schema SHA-256: 0ba7668ff3404962867d0a8ed0cf915f0e8d9ed74d1d559495e9d90a3622c89e.
// Do not edit manually.

/** Persistent, safe summary of a background job without source rows or local paths. */
export interface JobDescriptor {
  /** UUID v7 trace identity for the job mutation. */
  readonly correlation_id: string;
  /** ISO-8601 UTC timestamp when the job was created. */
  readonly created_at: string;
  /** UUID v7 immutable job identity. */
  readonly job_id: string;
  /** Stable job kind identifier. */
  readonly kind: string;
  /** Completed progress amount. */
  readonly progress_current: number;
  /** UUID v7 project identity that owns the job. */
  readonly project_id: string;
  /** Positive optimistic-concurrency revision. */
  readonly revision: number;
  /** Current lifecycle status such as QUEUED or RUNNING. */
  readonly status: string;
  /** ISO-8601 UTC timestamp of the latest job change. */
  readonly updated_at: string;
  /** Stable safe failure code. */
  readonly error_code?: string;
  /** Safe actionable failure message without raw errors. */
  readonly error_message?: string;
  /** Whether retrying the job is safe and permitted. */
  readonly error_retriable?: boolean;
  /** ISO-8601 UTC timestamp when execution finished. */
  readonly finished_at?: string;
  /** Safe Indonesian progress message without source data. */
  readonly progress_message?: string;
  /** Stable safe execution phase identifier. */
  readonly progress_phase?: string;
  /** Known total progress amount. */
  readonly progress_total?: number;
  /** Safe progress measurement unit. */
  readonly progress_unit?: string;
  /** ISO-8601 UTC timestamp when execution started. */
  readonly started_at?: string;
  readonly [additionalProperty: string]: unknown;
}
