import { useCallback, useEffect, useRef, useState } from "react";

import type { JobDescriptor, ProjectDescriptor } from "@teratai/contracts";

import type { JobClient, JobCursor } from "./job-client";
import {
  JOB_PAGE_SIZE,
  jobCenterError,
  mergeJobSnapshots,
  type JobCenterState,
} from "./job-center-model";

export interface JobCenterController extends JobCenterState {
  readonly cancel: (job: JobDescriptor) => Promise<void>;
  readonly clearError: () => void;
  readonly clearEventWarning: () => void;
  readonly loadMore: () => Promise<void>;
  readonly refresh: () => Promise<void>;
}

const EMPTY_STATE: JobCenterState = {
  cancelPendingIds: new Set<string>(),
  cursor: undefined,
  error: null,
  eventWarning: null,
  items: [],
  loadingMore: false,
  refreshing: false,
  status: "loading",
};

export function useJobCenter(
  client: JobClient | null,
  project: ProjectDescriptor,
): JobCenterController {
  const compatible = project.metadata_schema_version >= 2;
  const [state, setState] = useState<JobCenterState>(() => initialState(client, compatible));
  const generation = useRef(0);
  const mounted = useRef(true);
  const pendingCancellationIds = useRef(new Set<string>());

  const refresh = useCallback(async () => {
    if (client === null || !compatible) return;
    const requestGeneration = generation.current;
    setState((current) => ({
      ...current,
      error: null,
      refreshing: current.items.length > 0,
      status: current.items.length === 0 ? "loading" : "ready",
    }));
    try {
      const page = await client.list(JOB_PAGE_SIZE);
      if (!mounted.current || generation.current !== requestGeneration) return;
      setState((current) => ({
        ...current,
        ...(page.nextCursor === undefined ? { cursor: undefined } : { cursor: page.nextCursor }),
        error: null,
        items: mergeJobSnapshots([], page.items, project.project_id),
        refreshing: false,
        status: "ready",
      }));
    } catch (error: unknown) {
      if (!mounted.current || generation.current !== requestGeneration) return;
      const envelope = jobCenterError(error);
      setState((current) => ({
        ...current,
        error: envelope,
        refreshing: false,
        status: envelope.code === "PROJECT_UPGRADE_REQUIRED"
          ? "upgrade"
          : current.items.length === 0 ? "error" : "ready",
      }));
    }
  }, [client, compatible, project.project_id]);

  useEffect(() => {
    const effectGeneration = generation.current + 1;
    generation.current = effectGeneration;
    mounted.current = true;
    setState(initialState(client, compatible));
    if (client === null || !compatible) {
      return () => {
        mounted.current = false;
      };
    }

    let unlisten: (() => void) | undefined;
    void client.subscribe({
      onError(error) {
        if (!mounted.current || generation.current !== effectGeneration) return;
        setState((current) => ({
          ...current,
          eventWarning: error.message,
        }));
      },
      onEvent(event) {
        if (!mounted.current
          || generation.current !== effectGeneration
          || event.job.project_id !== project.project_id) return;
        setState((current) => ({
          ...current,
          items: mergeJobSnapshots(current.items, [event.job], project.project_id),
        }));
      },
    }).then((dispose) => {
      if (mounted.current && generation.current === effectGeneration) {
        unlisten = dispose;
      } else {
        dispose();
      }
    }).catch((error: unknown) => {
      if (!mounted.current || generation.current !== effectGeneration) return;
      setState((current) => ({
        ...current,
        eventWarning: jobCenterError(error).message,
      }));
    });
    void refresh();

    return () => {
      mounted.current = false;
      generation.current += 1;
      unlisten?.();
    };
  }, [client, compatible, project.project_id, refresh]);

  const loadMore = useCallback(async () => {
    if (client === null || state.cursor === undefined || state.loadingMore) return;
    const requestedCursor: JobCursor = state.cursor;
    const requestGeneration = generation.current;
    setState((current) => ({ ...current, error: null, loadingMore: true }));
    try {
      const page = await client.list(JOB_PAGE_SIZE, requestedCursor);
      if (!mounted.current || generation.current !== requestGeneration) return;
      setState((current) => ({
        ...current,
        ...(page.nextCursor === undefined ? { cursor: undefined } : { cursor: page.nextCursor }),
        items: mergeJobSnapshots(current.items, page.items, project.project_id),
        loadingMore: false,
      }));
    } catch (error: unknown) {
      if (!mounted.current || generation.current !== requestGeneration) return;
      setState((current) => ({
        ...current,
        error: jobCenterError(error),
        loadingMore: false,
      }));
    }
  }, [client, project.project_id, state.cursor, state.loadingMore]);

  const cancel = useCallback(async (job: JobDescriptor) => {
    if (client === null || pendingCancellationIds.current.has(job.job_id)) return;
    const requestGeneration = generation.current;
    pendingCancellationIds.current.add(job.job_id);
    setState((current) => ({
      ...current,
      cancelPendingIds: new Set([...current.cancelPendingIds, job.job_id]),
      error: null,
    }));
    try {
      const updated = await client.cancel(job);
      if (!mounted.current || generation.current !== requestGeneration) return;
      setState((current) => ({
        ...current,
        cancelPendingIds: withoutId(current.cancelPendingIds, job.job_id),
        items: mergeJobSnapshots(current.items, [updated], project.project_id),
      }));
    } catch (error: unknown) {
      if (mounted.current && generation.current === requestGeneration) {
        setState((current) => ({
          ...current,
          cancelPendingIds: withoutId(current.cancelPendingIds, job.job_id),
          error: jobCenterError(error),
        }));
      }
      let reconciled: JobDescriptor | null = null;
      try {
        reconciled = await client.get(job.job_id);
      } catch {
        // The original typed cancellation error is more actionable.
      }
      if (!mounted.current || generation.current !== requestGeneration) return;
      setState((current) => ({
        ...current,
        items: reconciled === null
          ? current.items
          : mergeJobSnapshots(current.items, [reconciled], project.project_id),
      }));
    } finally {
      pendingCancellationIds.current.delete(job.job_id);
    }
  }, [client, project.project_id]);

  return {
    ...state,
    cancel,
    clearError: useCallback(() => {
      setState((current) => ({ ...current, error: null }));
    }, []),
    clearEventWarning: useCallback(() => {
      setState((current) => ({ ...current, eventWarning: null }));
    }, []),
    loadMore,
    refresh,
  };
}

function initialState(client: JobClient | null, compatible: boolean): JobCenterState {
  if (!compatible) return { ...EMPTY_STATE, status: "upgrade" };
  if (client === null) return { ...EMPTY_STATE, status: "unavailable" };
  return EMPTY_STATE;
}

function withoutId(values: ReadonlySet<string>, id: string): ReadonlySet<string> {
  const next = new Set(values);
  next.delete(id);
  return next;
}
