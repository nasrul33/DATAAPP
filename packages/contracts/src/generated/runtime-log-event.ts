// Generated from packages/contracts/schemas/runtime-log-event.schema.json.
// Schema SHA-256: 10c920293561b971e8ee0e8df8bfb452ae512c0aaea25d21e547a2ec07f24fce.
// Do not edit manually.

/** Safe structured runtime log shared by desktop, native host, and Python engine layers. */
export interface RuntimeLogEvent {
  /** Stable component identifier without machine-specific paths. */
  readonly component: string;
  /** UUID v7 joining events for one end-to-end operation. */
  readonly correlation_id: string;
  /** Stable dot-separated event identifier. */
  readonly event: string;
  /** Runtime boundary that emitted the event: desktop, native, or engine. */
  readonly layer: string;
  /** Severity name: DEBUG, INFO, WARNING, or ERROR. */
  readonly level: string;
  /** User-safe diagnostic that excludes source data and secrets. */
  readonly message: string;
  /** Monotonic sequence within the correlated runtime exchange. */
  readonly sequence: number;
  /** ISO-8601 UTC timestamp ending in Z. */
  readonly timestamp: string;
  readonly [additionalProperty: string]: unknown;
}
