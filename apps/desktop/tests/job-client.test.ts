import { describe, expect, it } from "vitest";

import {
  JobClientError,
  parseJobDescriptor,
  parseJobLifecycleEvent,
  parseJobListResponse,
} from "../src/job/job-client";

const descriptor = {
  correlation_id: "00000000-0000-7000-8000-000000000403",
  created_at: "2026-07-24T01:00:00Z",
  job_id: "00000000-0000-7000-8000-000000000402",
  kind: "system.mock_long",
  progress_current: 2,
  progress_message: "Memproses pekerjaan.",
  progress_phase: "mock.work",
  progress_total: 10,
  progress_unit: "step",
  project_id: "00000000-0000-7000-8000-000000000401",
  revision: 3,
  started_at: "2026-07-24T01:01:00Z",
  status: "RUNNING",
  updated_at: "2026-07-24T01:02:00Z",
} as const;

describe("job client trust boundary", () => {
  it("accepts a bounded descriptor, page, and linked lifecycle event", () => {
    expect(parseJobDescriptor(descriptor)).toEqual(descriptor);
    expect(parseJobListResponse({
      items: [descriptor],
      next_cursor_job_id: descriptor.job_id,
      next_cursor_updated_at: descriptor.updated_at,
    })).toEqual({
      items: [descriptor],
      nextCursor: {
        jobId: descriptor.job_id,
        updatedAt: descriptor.updated_at,
      },
    });
    expect(parseJobLifecycleEvent({
      event_name: "job.progress",
      job: descriptor,
      occurred_at: descriptor.updated_at,
      protocol_version: "1.0",
      sequence: descriptor.revision,
    }).job).toEqual(descriptor);
  });

  it("rejects oversized pages, partial cursors, and unlinked event sequences", () => {
    expect(() => parseJobListResponse({
      items: Array.from({ length: 101 }, () => descriptor),
    })).toThrow(JobClientError);
    expect(() => parseJobListResponse({
      items: [descriptor],
      next_cursor_job_id: descriptor.job_id,
    })).toThrow(JobClientError);
    expect(() => parseJobLifecycleEvent({
      event_name: "job.progress",
      job: descriptor,
      occurred_at: descriptor.updated_at,
      protocol_version: "1.0",
      sequence: descriptor.revision + 1,
    })).toThrow(JobClientError);
  });

  it("rejects malformed status, revision, and optional fields", () => {
    expect(() => parseJobDescriptor({ ...descriptor, status: "UNKNOWN" })).toThrow(JobClientError);
    expect(() => parseJobDescriptor({ ...descriptor, revision: 0 })).toThrow(JobClientError);
    expect(() => parseJobDescriptor({ ...descriptor, error_retriable: "yes" })).toThrow(JobClientError);
  });
});
