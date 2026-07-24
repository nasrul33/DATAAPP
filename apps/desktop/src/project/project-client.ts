import { invoke, isTauri } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

import {
  createCorrelationId,
  type CorrelationRequest,
  type DesktopError,
  type ProjectCreateRequest,
  type ProjectDescriptor,
  type ProjectOpenRequest,
} from "@teratai/contracts";

const PROJECT_EXTENSION = "teratai";
const WINDOWS_RESERVED_NAME = /^(?:aux|con|nul|prn|com[1-9]|lpt[1-9])(?:\.|$)/i;
const WINDOWS_INVALID_FILE_CHARACTERS = '<>:"/\\|?*';

export interface ProjectClient {
  readonly close: () => Promise<void>;
  readonly create: (name: string, projectPath: string) => Promise<ProjectDescriptor>;
  readonly current: () => Promise<ProjectDescriptor | null>;
  readonly open: (projectPath: string) => Promise<ProjectDescriptor>;
  readonly pickCreatePath: (name: string) => Promise<string | null>;
  readonly pickOpenPath: () => Promise<string | null>;
  readonly upgrade: () => Promise<ProjectDescriptor>;
  readonly validate: (projectPath: string) => Promise<ProjectDescriptor>;
}

export class ProjectClientError extends Error {
  public constructor(public readonly envelope: DesktopError) {
    super(envelope.message);
    this.name = "ProjectClientError";
  }
}

export function projectRuntimeAvailable(): boolean {
  return isTauri();
}

export function suggestedProjectFileName(name: string): string {
  const normalized = name
    .normalize("NFKC")
    .split("")
    .map((character) => character.charCodeAt(0) < 32 || WINDOWS_INVALID_FILE_CHARACTERS.includes(character) ? "-" : character)
    .join("")
    .replaceAll(/\s+/g, " ")
    .replace(/[ .]+$/u, "")
    .trim();
  const safeName = normalized.length === 0 || WINDOWS_RESERVED_NAME.test(normalized)
    ? "Proyek Teratai"
    : normalized.slice(0, 100);
  return `${safeName}.${PROJECT_EXTENSION}`;
}

export const tauriProjectClient: ProjectClient = {
  async close() {
    await invokeProject<null>("project_close", correlationRequest());
  },
  async create(name, projectPath) {
    const request: ProjectCreateRequest = {
      name,
      project_id: createCorrelationId(),
      project_path: projectPath,
      request_id: createCorrelationId(),
    };
    return parseProjectDescriptor(await invokeProject("project_create", request));
  },
  async current() {
    const result = await invokeProject<unknown>("project_current", correlationRequest());
    return result === null ? null : parseProjectDescriptor(result);
  },
  async open(projectPath) {
    const request = openRequest(projectPath);
    return parseProjectDescriptor(await invokeProject("project_open", request));
  },
  async pickCreatePath(name) {
    return save({
      title: "Pilih lokasi proyek Teratai",
      defaultPath: suggestedProjectFileName(name),
      filters: [{ name: "Proyek Teratai", extensions: [PROJECT_EXTENSION] }],
    });
  },
  async pickOpenPath() {
    const selection = await open({
      title: "Buka proyek Teratai",
      directory: true,
      multiple: false,
    });
    return selection;
  },
  async upgrade() {
    return parseProjectDescriptor(
      await invokeProject("project_upgrade", correlationRequest()),
    );
  },
  async validate(projectPath) {
    const request = openRequest(projectPath);
    return parseProjectDescriptor(await invokeProject("project_validate", request));
  },
};

function correlationRequest(): CorrelationRequest {
  return { request_id: createCorrelationId() };
}

function openRequest(projectPath: string): ProjectOpenRequest {
  return {
    project_path: projectPath,
    request_id: createCorrelationId(),
  };
}

async function invokeProject<T>(command: string, request: object): Promise<T> {
  try {
    return await invoke<T>(command, { request });
  } catch (error: unknown) {
    throw new ProjectClientError(parseDesktopError(error));
  }
}

export function parseProjectDescriptor(value: unknown): ProjectDescriptor {
  if (!isRecord(value)
    || !isNonEmptyString(value.project_id)
    || !isNonEmptyString(value.name)
    || !isNonEmptyString(value.project_path)
    || !isNonEmptyString(value.created_at)
    || !isNonEmptyString(value.schema_version)
    || typeof value.metadata_schema_version !== "number"
    || !Number.isSafeInteger(value.metadata_schema_version)
    || value.metadata_schema_version < 1) {
    throw new ProjectClientError(fallbackError("Native project descriptor failed validation."));
  }
  return value as unknown as ProjectDescriptor;
}

export function parseDesktopError(value: unknown): DesktopError {
  if (!isRecord(value)
    || !isNonEmptyString(value.code)
    || !isNonEmptyString(value.message)
    || !isNonEmptyString(value.detail)
    || typeof value.retriable !== "boolean"
    || !isNonEmptyString(value.correlation_id)
    || !Array.isArray(value.field_errors)
    || !value.field_errors.every((item) => typeof item === "string")
    || !(value.remediation === undefined || value.remediation === null || typeof value.remediation === "string")) {
    return fallbackError("Native project command returned an invalid error envelope.");
  }
  return value as unknown as DesktopError;
}

function fallbackError(detail: string): DesktopError {
  return {
    code: "OPERATION_FAILED",
    correlation_id: createCorrelationId(),
    detail,
    field_errors: [],
    message: "Operasi proyek tidak dapat diselesaikan.",
    remediation: "Mulai ulang Teratai lalu coba kembali.",
    retriable: true,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}
