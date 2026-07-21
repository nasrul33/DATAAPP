import { describe, expect, it } from "vitest";

import type {
  ContractMetadata,
  CorrelationRequest,
  DesktopError,
  EngineError,
  EngineHandshakeRequest,
  EngineHandshakeResponse,
  JobDescriptor,
  JobEnqueueRequest,
  JobFailureRequest,
  JobProgressUpdateRequest,
  JobTransitionRequest,
  ProjectCreateRequest,
  ProjectDescriptor,
  ProjectManifest,
  ProjectOpenRequest,
  RuntimeLogEvent,
} from "../src/index";
import { createCorrelationId, createRuntimeLogEvent } from "../src/index";

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

describe("project lifecycle contracts", () => {
  it("keeps create, manifest, and descriptor identities aligned", () => {
    const request = {
      name: "Audit Belanja 2026",
      project_id: "00000000-0000-7000-8000-000000000100",
      project_path: "D:\\Projects\\Audit Belanja 2026.teratai",
      request_id: "00000000-0000-7000-8000-000000000101",
    } satisfies ProjectCreateRequest;
    const manifest = {
      app_version: "0.1.0",
      created_at: "2026-07-20T12:00:00Z",
      metadata_schema_version: 1,
      name: request.name,
      project_id: request.project_id,
      schema_version: "1.0.0",
    } satisfies ProjectManifest;
    const descriptor = {
      created_at: manifest.created_at,
      metadata_schema_version: manifest.metadata_schema_version,
      name: manifest.name,
      project_id: manifest.project_id,
      project_path: request.project_path,
      schema_version: manifest.schema_version,
    } satisfies ProjectDescriptor;

    expect(descriptor.project_id).toBe(request.project_id);
    expect(descriptor.metadata_schema_version).toBe(1);
  });

  it("represents typed desktop project commands and failures", () => {
    const correlation = {
      request_id: "00000000-0000-7000-8000-000000000111",
    } satisfies CorrelationRequest;
    const open = {
      project_path: "D:\\Projects\\Audit Belanja 2026.teratai",
      request_id: correlation.request_id,
    } satisfies ProjectOpenRequest;
    const error = {
      code: "PERMISSION_DENIED",
      correlation_id: open.request_id,
      detail: "operating system denied project path access",
      field_errors: [],
      message: "Teratai tidak memiliki izin untuk lokasi tersebut.",
      remediation: "Pilih lokasi lain.",
      retriable: true,
    } satisfies DesktopError;

    expect(error.correlation_id).toBe(correlation.request_id);
    expect(error.code).toBe("PERMISSION_DENIED");
  });
});

describe("runtime logging", () => {
  it("creates deterministic UUID v7 correlation identifiers", () => {
    const correlationId = createCorrelationId({
      randomBytes: new Uint8Array(16).fill(0xab),
      timestampMs: 1_721_469_600_000,
    });

    expect(correlationId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(correlationId[14]).toBe("7");
  });

  it("creates a canonical safe structured event", () => {
    const event = createRuntimeLogEvent({
      component: "desktop-shell",
      correlationId: "00000000-0000-7000-8000-000000000007",
      event: "desktop.startup",
      layer: "desktop",
      level: "INFO",
      message: "Desktop shell dimulai.",
      sequence: 1,
      timestamp: new Date("2026-07-20T10:00:00.000Z"),
    }) satisfies RuntimeLogEvent;

    expect(event.timestamp).toBe("2026-07-20T10:00:00.000Z");
    expect(event.correlation_id).toMatch(/-7[0-9a-f]{3}-/);
  });

  it("rejects an invalid correlation identifier at runtime", () => {
    expect(() => createRuntimeLogEvent({
      component: "desktop-shell",
      correlationId: "not-a-uuid",
      event: "desktop.startup",
      layer: "desktop",
      level: "INFO",
      message: "Desktop shell dimulai.",
      sequence: 1,
    })).toThrow("correlationId must be a lowercase UUID v7");
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

describe("job contracts", () => {
  it("represents persistent job lifecycle payloads", () => {
    const descriptor = {
      correlation_id: "00000000-0000-7000-8000-000000000211",
      created_at: "2026-07-20T12:00:00Z",
      job_id: "00000000-0000-7000-8000-000000000211",
      kind: "system.mock_long",
      progress_current: 0,
      progress_message: "Pekerjaan menunggu untuk diproses.",
      progress_phase: "queued",
      progress_total: 100,
      progress_unit: "step",
      project_id: "00000000-0000-7000-8000-000000000210",
      revision: 1,
      status: "QUEUED",
      updated_at: "2026-07-20T12:00:00Z",
    } satisfies JobDescriptor;
    const enqueue = {
      correlation_id: "00000000-0000-7000-8000-000000000212",
      job_id: "00000000-0000-7000-8000-000000000212",
      kind: descriptor.kind,
      progress_total: 100,
      progress_unit: "step",
    } satisfies JobEnqueueRequest;
    const transition = {
      correlation_id: "00000000-0000-7000-8000-000000000213",
      expected_revision: 1,
      job_id: "00000000-0000-7000-8000-000000000213",
    } satisfies JobTransitionRequest;
    const progress = {
      correlation_id: "00000000-0000-7000-8000-000000000214",
      current: 0,
      expected_revision: 1,
      job_id: "00000000-0000-7000-8000-000000000214",
      message: "Pekerjaan sedang menyiapkan langkah pertama.",
      phase: "queued",
      total: 100,
      unit: "step",
    } satisfies JobProgressUpdateRequest;
    const failure = {
      correlation_id: "00000000-0000-7000-8000-000000000215",
      error_code: "JOB_EXECUTION_FAILED",
      error_message: "Pekerjaan tidak dapat diselesaikan. Silakan coba lagi.",
      error_retriable: true,
      expected_revision: 1,
      job_id: "00000000-0000-7000-8000-000000000215",
    } satisfies JobFailureRequest;

    expect(descriptor.status).toBe("QUEUED");
    expect(enqueue.progress_total).toBe(100);
    expect(transition.expected_revision).toBe(1);
    expect(progress.current).toBe(0);
    expect(failure.error_retriable).toBe(true);
  });
});
