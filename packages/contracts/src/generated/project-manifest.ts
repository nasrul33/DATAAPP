// Generated from packages/contracts/schemas/project-manifest.schema.json.
// Schema SHA-256: afdf6fb9906036421d3a461049ae9bb5f0616709af2c9a273975cbd441843db4.
// Do not edit manually.

/** Immutable identity and compatibility metadata stored at a Teratai project root. */
export interface ProjectManifest {
  /** Teratai version that created the project. */
  readonly app_version: string;
  /** ISO-8601 UTC creation timestamp. */
  readonly created_at: string;
  /** Required `SQLite` metadata schema version. */
  readonly metadata_schema_version: number;
  /** Human-readable project name. */
  readonly name: string;
  /** UUID v7 immutable project identity. */
  readonly project_id: string;
  /** Project manifest semantic version. */
  readonly schema_version: string;
  readonly [additionalProperty: string]: unknown;
}
