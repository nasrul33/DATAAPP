import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { DesktopError, JobDescriptor } from "@teratai/contracts";

import { JobCenterView } from "../src/job/job-center";
import {
  canCancelJob,
  filterJobs,
  jobProgressPercent,
  MAX_VISIBLE_JOBS,
  mergeJobSnapshots,
} from "../src/job/job-center-model";
import type { JobCenterController } from "../src/job/use-job-center";

const PROJECT_ID = "00000000-0000-7000-8000-000000000501";

describe("job center model", () => {
  it("merges only newer project-scoped revisions and keeps newest-first order", () => {
    const older = job({ job_id: uuid(2), revision: 2, updated_at: "2026-07-24T01:02:00Z" });
    const stale = job({ job_id: uuid(2), progress_current: 1, revision: 1, updated_at: "2026-07-24T01:01:00Z" });
    const newer = job({ job_id: uuid(3), revision: 1, updated_at: "2026-07-24T01:03:00Z" });
    const foreign = job({ job_id: uuid(4), project_id: uuid(99), updated_at: "2026-07-24T01:04:00Z" });

    expect(mergeJobSnapshots([older], [stale, newer, foreign], PROJECT_ID)).toEqual([newer, older]);
  });

  it("bounds retained snapshots and filters the loaded set deterministically", () => {
    const jobs = Array.from({ length: MAX_VISIBLE_JOBS + 5 }, (_, index) => job({
      job_id: uuid(index + 10),
      status: index % 2 === 0 ? "RUNNING" : "FAILED",
      updated_at: `2026-07-24T01:${String(index).padStart(2, "0")}:00Z`,
    }));
    const merged = mergeJobSnapshots([], jobs, PROJECT_ID);

    expect(merged).toHaveLength(MAX_VISIBLE_JOBS);
    expect(filterJobs(merged, "FAILED").every((item) => item.status === "FAILED")).toBe(true);
  });

  it("exposes cancellation and progress only for valid lifecycle data", () => {
    expect(canCancelJob(job({ status: "QUEUED" }))).toBe(true);
    expect(canCancelJob(job({ status: "RUNNING" }))).toBe(true);
    expect(canCancelJob(job({ status: "CANCELLING" }))).toBe(false);
    expect(jobProgressPercent(job({ progress_current: 3, progress_total: 4 }))).toBe(75);
    expect(jobProgressPercent(job())).toBeNull();
  });
});

describe("job center UI states", () => {
  it("renders loading, upgrade, unavailable, and initial error states", () => {
    expect(render("loading")).toContain('aria-busy="true"');
    expect(render("upgrade")).toContain("tidak di-upgrade otomatis");
    expect(render("unavailable")).toContain("hanya tersedia di aplikasi desktop");
    expect(render("error", desktopError())).toContain("Coba lagi");
  });

  it("renders empty and fully labelled lifecycle rows", () => {
    expect(render("ready")).toContain("Belum ada pekerjaan");

    const running = job({
      progress_current: 4,
      progress_message: "Membaca batch aman.",
      progress_total: 10,
      status: "RUNNING",
    });
    const failed = job({
      error_code: "RESOURCE_EXHAUSTED",
      error_message: "Budget memori tidak mencukupi.",
      job_id: uuid(7),
      status: "FAILED",
      updated_at: "2026-07-24T01:05:00Z",
    });
    const markup = renderToStaticMarkup(
      <JobCenterView controller={controller("ready", null, [running, failed])} />,
    );

    expect(markup).toContain("Job Center");
    expect(markup).toContain("Berjalan");
    expect(markup).toContain("Gagal");
    expect(markup).toContain("Membaca batch aman");
    expect(markup).toContain('value="40"');
    expect(markup).toContain("Batalkan");
    expect(markup).toContain("Budget memori tidak mencukupi");
    expect(markup).toContain("Tidak ada aksi");
  });

  it("keeps durable rows visible while surfacing refresh and event errors", () => {
    const state = controller("ready", desktopError(), [job({ status: "SUCCEEDED" })], {
      eventWarning: "invalid event",
      refreshing: true,
    });
    const markup = renderToStaticMarkup(<JobCenterView controller={state} />);

    expect(markup).toContain("Menyinkronkan");
    expect(markup).toContain("Data pekerjaan tidak dapat dimuat");
    expect(markup).toContain("Pembaruan langsung terputus");
    expect(markup).toContain("Selesai");
  });
});

function render(status: JobCenterController["status"], error: DesktopError | null = null): string {
  return renderToStaticMarkup(<JobCenterView controller={controller(status, error)} />);
}

function controller(
  status: JobCenterController["status"],
  error: DesktopError | null,
  items: readonly JobDescriptor[] = [],
  overrides: Partial<JobCenterController> = {},
): JobCenterController {
  return {
    cancel: () => Promise.resolve(),
    cancelPendingIds: new Set<string>(),
    clearError: () => undefined,
    clearEventWarning: () => undefined,
    cursor: undefined,
    error,
    eventWarning: null,
    items,
    loadMore: () => Promise.resolve(),
    loadingMore: false,
    refresh: () => Promise.resolve(),
    refreshing: false,
    status,
    ...overrides,
  };
}

function job(overrides: Partial<JobDescriptor> = {}): JobDescriptor {
  return {
    correlation_id: uuid(3),
    created_at: "2026-07-24T01:00:00Z",
    job_id: uuid(2),
    kind: "system.mock_long",
    progress_current: 0,
    project_id: PROJECT_ID,
    revision: 1,
    status: "QUEUED",
    updated_at: "2026-07-24T01:00:00Z",
    ...overrides,
  };
}

function uuid(value: number): string {
  return `00000000-0000-7000-8000-${String(value).padStart(12, "0")}`;
}

function desktopError(): DesktopError {
  return {
    code: "OPERATION_FAILED",
    correlation_id: uuid(8),
    detail: "job list failed",
    field_errors: [],
    message: "Data pekerjaan tidak dapat dimuat.",
    remediation: "Coba muat ulang.",
    retriable: true,
  };
}
