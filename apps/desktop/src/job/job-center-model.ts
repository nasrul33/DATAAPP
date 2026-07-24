import type { DesktopError, JobDescriptor } from "@teratai/contracts";

import { JobClientError } from "./job-client";

export const JOB_PAGE_SIZE = 25;
export const MAX_VISIBLE_JOBS = 100;

export const JOB_STATUS_OPTIONS = [
  "ALL",
  "QUEUED",
  "RUNNING",
  "CANCELLING",
  "SUCCEEDED",
  "FAILED",
  "CANCELLED",
] as const;

export type JobStatusFilter = (typeof JOB_STATUS_OPTIONS)[number];
export type JobCenterStatus = "error" | "loading" | "ready" | "unavailable" | "upgrade";

export interface JobCenterState {
  readonly cancelPendingIds: ReadonlySet<string>;
  readonly cursor: {
    readonly jobId: string;
    readonly updatedAt: string;
  } | undefined;
  readonly error: DesktopError | null;
  readonly eventWarning: string | null;
  readonly items: readonly JobDescriptor[];
  readonly loadingMore: boolean;
  readonly refreshing: boolean;
  readonly status: JobCenterStatus;
}

export function mergeJobSnapshots(
  current: readonly JobDescriptor[],
  incoming: readonly JobDescriptor[],
  projectId: string,
): readonly JobDescriptor[] {
  const byId = new Map<string, JobDescriptor>();
  for (const job of current) {
    if (job.project_id === projectId) byId.set(job.job_id, job);
  }
  for (const job of incoming) {
    if (job.project_id !== projectId) continue;
    const existing = byId.get(job.job_id);
    if (existing === undefined || job.revision > existing.revision) {
      byId.set(job.job_id, job);
    }
  }
  return [...byId.values()]
    .sort(compareJobsNewestFirst)
    .slice(0, MAX_VISIBLE_JOBS);
}

export function filterJobs(
  items: readonly JobDescriptor[],
  status: JobStatusFilter,
): readonly JobDescriptor[] {
  return status === "ALL" ? items : items.filter((job) => job.status === status);
}

export function canCancelJob(job: JobDescriptor): boolean {
  return job.status === "QUEUED" || job.status === "RUNNING";
}

export function jobProgressPercent(job: JobDescriptor): number | null {
  if (job.progress_total === undefined || job.progress_total <= 0) return null;
  return Math.min(100, Math.round((job.progress_current / job.progress_total) * 100));
}

export function jobCenterError(error: unknown): DesktopError {
  if (error instanceof JobClientError && error.causeEnvelope !== undefined) {
    return error.causeEnvelope;
  }
  return {
    code: "OPERATION_FAILED",
    correlation_id: "00000000-0000-7000-8000-000000000000",
    detail: "desktop job center failed unexpectedly",
    field_errors: [],
    message: "Data pekerjaan tidak dapat dimuat.",
    remediation: "Coba muat ulang. Jika masalah berulang, mulai ulang Teratai.",
    retriable: true,
  };
}

function compareJobsNewestFirst(left: JobDescriptor, right: JobDescriptor): number {
  const timeOrder = right.updated_at.localeCompare(left.updated_at);
  return timeOrder === 0 ? right.job_id.localeCompare(left.job_id) : timeOrder;
}
