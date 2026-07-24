// Generated from packages/contracts/schemas/job-get-request.schema.json.
// Schema SHA-256: 1b708a1b590439a9b73195ef94cb9ab049100e2d0c2352c90815c96b3acf9680.
// Do not edit manually.

/** Typed desktop request for one project-scoped persistent job snapshot. */
export interface JobGetRequest {
  /** Lowercase UUID v7 identity for this desktop request. */
  readonly correlation_id: string;
  /** Lowercase UUID v7 identity of the requested job. */
  readonly job_id: string;
  readonly [additionalProperty: string]: unknown;
}
