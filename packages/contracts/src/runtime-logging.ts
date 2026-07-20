import type { RuntimeLogEvent } from "./generated/runtime-log-event";

const MAX_UUID_TIMESTAMP = 0xffff_ffff_ffff;
const UUID_V7_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

export interface CorrelationIdOptions {
  readonly randomBytes?: Uint8Array;
  readonly timestampMs?: number;
}

export interface RuntimeLogInput {
  readonly component: string;
  readonly correlationId: string;
  readonly event: string;
  readonly layer: "desktop" | "engine" | "native";
  readonly level: "DEBUG" | "ERROR" | "INFO" | "WARNING";
  readonly message: string;
  readonly sequence: number;
  readonly timestamp?: Date;
}

/** Create a UUID v7 suitable for operation correlation without an external dependency. */
export function createCorrelationId(options: CorrelationIdOptions = {}): string {
  const timestampMs = options.timestampMs ?? Date.now();
  if (!Number.isSafeInteger(timestampMs) || timestampMs < 0 || timestampMs > MAX_UUID_TIMESTAMP) {
    throw new RangeError("timestampMs must be an unsigned 48-bit integer");
  }

  const bytes = options.randomBytes === undefined
    ? globalThis.crypto.getRandomValues(new Uint8Array(16))
    : Uint8Array.from(options.randomBytes);
  if (bytes.length !== 16) throw new RangeError("randomBytes must contain exactly 16 bytes");

  let remainingTimestamp = timestampMs;
  for (let index = 5; index >= 0; index -= 1) {
    bytes[index] = remainingTimestamp & 0xff;
    remainingTimestamp = Math.floor(remainingTimestamp / 256);
  }
  bytes[6] = (bytes[6] ?? 0) & 0x0f | 0x70;
  bytes[8] = (bytes[8] ?? 0) & 0x3f | 0x80;

  const hexadecimal = Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");
  return `${hexadecimal.slice(0, 8)}-${hexadecimal.slice(8, 12)}-${hexadecimal.slice(12, 16)}-${hexadecimal.slice(16, 20)}-${hexadecimal.slice(20)}`;
}

/** Build a safe canonical runtime event; callers must not place source data in message. */
export function createRuntimeLogEvent(input: RuntimeLogInput): RuntimeLogEvent {
  if (!Number.isSafeInteger(input.sequence) || input.sequence < 1) {
    throw new RangeError("sequence must be a positive integer");
  }
  if ([input.component, input.correlationId, input.event, input.message].some((value) => value.trim().length === 0)) {
    throw new TypeError("runtime log string fields cannot be empty");
  }
  if (!UUID_V7_PATTERN.test(input.correlationId)) {
    throw new TypeError("correlationId must be a lowercase UUID v7");
  }

  const timestamp = input.timestamp ?? new Date();
  if (Number.isNaN(timestamp.getTime())) throw new RangeError("timestamp must be a valid Date");

  return {
    component: input.component,
    correlation_id: input.correlationId,
    event: input.event,
    layer: input.layer,
    level: input.level,
    message: input.message,
    sequence: input.sequence,
    timestamp: timestamp.toISOString(),
  };
}
