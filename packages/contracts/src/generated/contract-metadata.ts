// Generated from packages/contracts/schemas/contract-metadata.schema.json.
// Schema SHA-256: db8b3860cc8a21ef8174d42662658f12d4626b608156c4b2494470025c748056.
// Do not edit manually.

/** Version metadata embedded in generated Teratai cross-language contracts. */
export interface ContractMetadata {
  /** Monotonic revision of the generator contract subset. */
  readonly generator_revision: number;
  /** Major and minor engine protocol version. */
  readonly protocol_version: string;
  /** Stable kebab-case canonical schema name. */
  readonly schema_name: string;
  /** Semantic version of this schema. */
  readonly schema_version: string;
  readonly [additionalProperty: string]: unknown;
}
