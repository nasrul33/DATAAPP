import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  createCorrelationId,
  type JobDescriptor,
  type JobGetRequest,
  type JobLifecycleEvent,
  type JobListRequest,
  type JobTransitionRequest,
} from "@teratai/contracts";

import { parseDesktopError } from "../project/project-client";

const JOB_LIFECYCLE_CHANNEL = "job:lifecycle";
const JOB_STATUSES = new Set([
  "QUEUED",
  "RUNNING",
  "SUCCEEDED",
  "FAILED",
  "CANCELLING",
  "CANCELLED",
]);
const JOB_EVENT_NAMES = new Set([
  "job.queued",
  "job.started",
  "job.progress",
  "job.warning",
  "job.completed",
  "job.failed",
  "job.cancellation_requested",
  "job.cancelled",
]);

export interface JobCursor {
  readonly jobId: string;
  readonly updatedAt: string;
}

export interface JobPage {
  readonly items: readonly JobDescriptor[];
  readonly nextCursor?: JobCursor;
}

export interface JobEventHandlers {
  readonly onError: (error: JobClientError) => void;
  readonly onEvent: (event: JobLifecycleEvent) => void;
}

export interface JobClient {
  readonly cancel: (job: JobDescriptor) => Promise<JobDescriptor>;
  readonly get: (jobId: string) => Promise<JobDescriptor>;
  readonly list: (limit: number, cursor?: JobCursor) => Promise<JobPage>;
  readonly subscribe: (handlers: JobEventHandlers) => Promise<UnlistenFn>;
}

export class JobClientError extends Error {
  public constructor(
    message: string,
    public readonly causeEnvelope?: ReturnType<typeof parseDesktopError>,
  ) {
    super(message);
    this.name = "JobClientError";
  }
}

export const tauriJobClient: JobClient = {
  async cancel(job) {
    const request: JobTransitionRequest = {
      correlation_id: createCorrelationId(),
      expected_revision: job.revision,
      job_id: job.job_id,
    };
    return parseJobDescriptor(await invokeJob("job_cancel", request));
  },
  async get(jobId) {
    const request: JobGetRequest = {
      correlation_id: createCorrelationId(),
      job_id: jobId,
    };
    return parseJobDescriptor(await invokeJob("job_get", request));
  },
  async list(limit, cursor) {
    const request: JobListRequest = {
      correlation_id: createCorrelationId(),
      limit,
      ...(cursor === undefined
        ? {}
        : {
            cursor_job_id: cursor.jobId,
            cursor_updated_at: cursor.updatedAt,
          }),
    };
    return parseJobListResponse(await invokeJob("job_list", request));
  },
  async subscribe(handlers) {
    return listen<unknown>(JOB_LIFECYCLE_CHANNEL, (event) => {
      try {
        handlers.onEvent(parseJobLifecycleEvent(event.payload));
      } catch (error: unknown) {
        handlers.onError(toClientError(error, "Native job event failed validation."));
      }
    });
  },
};

async function invokeJob(command: string, request: object): Promise<unknown> {
  try {
    return await invoke<unknown>(command, { request });
  } catch (error: unknown) {
    const envelope = parseDesktopError(error);
    throw new JobClientError(envelope.message, envelope);
  }
}

export function parseJobListResponse(value: unknown): JobPage {
  if (!isRecord(value) || !Array.isArray(value.items) || value.items.length > 100) {
    throw new JobClientError("Native job page failed validation.");
  }
  const items = value.items.map(parseJobDescriptor);
  const cursorUpdatedAt = value.next_cursor_updated_at;
  const cursorJobId = value.next_cursor_job_id;
  const hasNoCursor = cursorUpdatedAt === undefined || cursorUpdatedAt === null;
  const hasNoJobId = cursorJobId === undefined || cursorJobId === null;
  if (hasNoCursor !== hasNoJobId) {
    throw new JobClientError("Native job page cursor failed validation.");
  }
  if (hasNoCursor && hasNoJobId) return { items };
  if (!isNonEmptyString(cursorUpdatedAt) || !isUuidV7(cursorJobId)) {
    throw new JobClientError("Native job page cursor failed validation.");
  }
  return {
    items,
    nextCursor: {
      jobId: cursorJobId,
      updatedAt: cursorUpdatedAt,
    },
  };
}

export function parseJobLifecycleEvent(value: unknown): JobLifecycleEvent {
  if (!isRecord(value)
    || value.protocol_version !== "1.0"
    || !isNonEmptyString(value.event_name)
    || !JOB_EVENT_NAMES.has(value.event_name)
    || !isPositiveInteger(value.sequence)
    || !isNonEmptyString(value.occurred_at)) {
    throw new JobClientError("Native job event failed validation.");
  }
  const job = parseJobDescriptor(value.job);
  if (value.sequence !== job.revision || value.occurred_at !== job.updated_at) {
    throw new JobClientError("Native job event sequence failed validation.");
  }
  return { ...value, job } as JobLifecycleEvent;
}

export function parseJobDescriptor(value: unknown): JobDescriptor {
  if (!isRecord(value)
    || !isUuidV7(value.job_id)
    || !isUuidV7(value.project_id)
    || !isUuidV7(value.correlation_id)
    || !isNonEmptyString(value.kind)
    || !isNonEmptyString(value.status)
    || !JOB_STATUSES.has(value.status)
    || !isPositiveInteger(value.revision)
    || !isNonEmptyString(value.created_at)
    || !isNonEmptyString(value.updated_at)
    || !isNonNegativeInteger(value.progress_current)
    || !isOptionalString(value.started_at)
    || !isOptionalString(value.finished_at)
    || !isOptionalPositiveInteger(value.progress_total)
    || !isOptionalString(value.progress_unit)
    || !isOptionalString(value.progress_phase)
    || !isOptionalString(value.progress_message)
    || !isOptionalString(value.error_code)
    || !isOptionalString(value.error_message)
    || !isOptionalBoolean(value.error_retriable)) {
    throw new JobClientError("Native job descriptor failed validation.");
  }
  return value as JobDescriptor;
}

function toClientError(error: unknown, fallback: string): JobClientError {
  return error instanceof JobClientError ? error : new JobClientError(fallback);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}

function isNonNegativeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isOptionalString(value: unknown): boolean {
  return value === undefined || value === null || isNonEmptyString(value);
}

function isOptionalPositiveInteger(value: unknown): boolean {
  return value === undefined || value === null || isPositiveInteger(value);
}

function isOptionalBoolean(value: unknown): boolean {
  return value === undefined || value === null || typeof value === "boolean";
}

function isUuidV7(value: unknown): value is string {
  return typeof value === "string"
    && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value);
}
