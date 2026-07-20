import { describe, expect, it } from "vitest";

import type {
  ContractMetadata,
  EngineError,
  EngineHandshakeRequest,
  EngineHandshakeResponse,
} from "../src/index";

describe("generated contract metadata", () => {
  it("round-trips the canonical fixture shape", () => {
    const metadata = {
      generator_revision: 1,
      protocol_version: "1.0",
      schema_name: "contract-metadata",
      schema_version: "1.0.0",
    } satisfies ContractMetadata;

    expect(JSON.parse(JSON.stringify(metadata))).toEqual(metadata);
  });
});

describe("engine handshake contracts", () => {
  it("represents the complete typed handshake exchange", () => {
    const request = {
      command: "engine.handshake",
      host_version: "0.1.0",
      protocol_version: "1.0",
      request_id: "request-6",
    } satisfies EngineHandshakeRequest;
    const response = {
      capabilities: ["health"],
      engine_version: "0.1.0",
      healthy: true,
      protocol_version: request.protocol_version,
      python_version: "3.12.6",
      request_id: request.request_id,
      status: "ready",
    } satisfies EngineHandshakeResponse;
    const error = {
      code: "ENGINE_UNAVAILABLE",
      correlation_id: request.request_id,
      detail: "protocol mismatch",
      field_errors: ["protocol_version tidak didukung"],
      message: "Versi protokol engine tidak kompatibel.",
      retriable: false,
    } satisfies EngineError;

    expect(response.healthy).toBe(true);
    expect(error.correlation_id).toBe(request.request_id);
  });
});
