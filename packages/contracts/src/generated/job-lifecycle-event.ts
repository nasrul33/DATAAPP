// Generated from packages/contracts/schemas/job-lifecycle-event.schema.json.
// Schema SHA-256: faf0049de1532cfdb546132c164d093e90d3ccd27ad3f871531d0e9e3cb59453.
// Do not edit manually.

import type { JobDescriptor } from "./job-descriptor";

/** Best-effort desktop notification backed by the durable job snapshot and revision. */
export interface JobLifecycleEvent {
  /** Stable lifecycle event name such as job.progress or job.failed. */
  readonly event_name: string;
  /** Trusted persistent snapshot after the lifecycle mutation. */
  readonly job: JobDescriptor;
  /** UTC timestamp of the persisted lifecycle mutation. */
  readonly occurred_at: string;
  /** Desktop job event protocol version. */
  readonly protocol_version: string;
  /** Positive per-job sequence equal to the persisted snapshot revision. */
  readonly sequence: number;
  readonly [additionalProperty: string]: unknown;
}
