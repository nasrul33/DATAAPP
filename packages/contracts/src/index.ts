export const ENGINE_PROTOCOL_VERSION = "1.0" as const;

export type { ContractMetadata } from "./generated/contract-metadata";
export type { EngineError } from "./generated/engine-error";
export type { EngineHandshakeRequest } from "./generated/engine-handshake-request";
export type { EngineHandshakeResponse } from "./generated/engine-handshake-response";
export type { RuntimeLogEvent } from "./generated/runtime-log-event";
export {
  createCorrelationId,
  createRuntimeLogEvent,
  type RuntimeLogInput,
} from "./runtime-logging";
