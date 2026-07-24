// Generated from packages/contracts/schemas/job-list-response.schema.json.
// Schema SHA-256: 8d85ee15c87acb87cf5861195c51194307708b77ca4640025d50ac1f4ffa1698.
// Do not edit manually.

import type { JobDescriptor } from "./job-descriptor";

/** Bounded project-scoped job page with an optional opaque keyset cursor pair. */
export interface JobListResponse {
  /** Trusted persistent job snapshots ordered newest first. */
  readonly items: readonly JobDescriptor[];
  /** Lowercase UUID v7 for the next keyset page. */
  readonly next_cursor_job_id?: string;
  /** UTC timestamp for the next keyset page. */
  readonly next_cursor_updated_at?: string;
  readonly [additionalProperty: string]: unknown;
}
