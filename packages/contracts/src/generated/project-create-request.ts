// Generated from packages/contracts/schemas/project-create-request.schema.json.
// Schema SHA-256: a933ec6c79528f5e9a87ce7e8dc72e0e9202ec6189e1a4ea63db34f203a055d9.
// Do not edit manually.

/** Validated native request for creating one local project directory. */
export interface ProjectCreateRequest {
  /** Human-readable project name. */
  readonly name: string;
  /** UUID v7 identity assigned to the new project. */
  readonly project_id: string;
  /** Absolute user-approved target directory ending in .teratai. */
  readonly project_path: string;
  /** UUID v7 correlation identifier. */
  readonly request_id: string;
  readonly [additionalProperty: string]: unknown;
}
