// Generated from packages/contracts/schemas/project-descriptor.schema.json.
// Schema SHA-256: 7f0e216e6cc84a1210efea34a63192bc9016da8eedb92044854c752023777392.
// Do not edit manually.

/** Validated project identity returned by the native project core. */
export interface ProjectDescriptor {
  /** ISO-8601 UTC creation timestamp. */
  readonly created_at: string;
  /** Validated `SQLite` metadata schema version. */
  readonly metadata_schema_version: number;
  /** Human-readable project name. */
  readonly name: string;
  /** UUID v7 immutable project identity. */
  readonly project_id: string;
  /** Canonical absolute project directory. */
  readonly project_path: string;
  /** Validated project manifest semantic version. */
  readonly schema_version: string;
  readonly [additionalProperty: string]: unknown;
}
