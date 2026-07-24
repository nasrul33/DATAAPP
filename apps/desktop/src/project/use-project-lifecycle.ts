import { useCallback, useEffect, useMemo, useState } from "react";

import type { DesktopError, ProjectDescriptor } from "@teratai/contracts";

import { ProjectClientError, type ProjectClient } from "./project-client";

export type ProjectViewStatus = "active" | "empty" | "error" | "loading" | "permission";
export type ProjectActionStatus = "closing" | "creating" | "idle" | "opening" | "selecting" | "upgrading";

export interface ProjectLifecycleState {
  readonly actionStatus: ProjectActionStatus;
  readonly error: DesktopError | null;
  readonly project: ProjectDescriptor | null;
  readonly status: ProjectViewStatus;
}

export interface ProjectLifecycle extends ProjectLifecycleState {
  readonly closeProject: () => Promise<void>;
  readonly createProject: (name: string) => Promise<boolean>;
  readonly dismissError: () => void;
  readonly openProject: () => Promise<boolean>;
  readonly retry: () => Promise<void>;
  readonly upgradeProject: () => Promise<boolean>;
}

export type StartupStateOverride = "error" | "loading" | "ready";

export function useProjectLifecycle(
  client: ProjectClient | null,
  startupOverride?: StartupStateOverride,
): ProjectLifecycle {
  const initialState = useMemo(
    () => initialProjectState(client, startupOverride),
    [client, startupOverride],
  );
  const [state, setState] = useState<ProjectLifecycleState>(initialState);

  const loadCurrent = useCallback(async () => {
    if (client === null) {
      setState(permissionState());
      return;
    }
    setState((current) => ({ ...current, actionStatus: "idle", error: null, status: "loading" }));
    try {
      const project = await client.current();
      setState({ actionStatus: "idle", error: null, project, status: project === null ? "empty" : "active" });
    } catch (error: unknown) {
      setState(errorState(error));
    }
  }, [client]);

  useEffect(() => {
    if (startupOverride === undefined) void loadCurrent();
  }, [loadCurrent, startupOverride]);

  const createProject = useCallback(async (name: string) => {
    if (client === null) return false;
    setState((current) => ({ ...current, actionStatus: "selecting", error: null }));
    try {
      const projectPath = await client.pickCreatePath(name);
      if (projectPath === null) {
        setState((current) => ({ ...current, actionStatus: "idle" }));
        return false;
      }
      setState((current) => ({ ...current, actionStatus: "creating" }));
      const project = await client.create(name, projectPath);
      setState({ actionStatus: "idle", error: null, project, status: "active" });
      return true;
    } catch (error: unknown) {
      setState(errorState(error));
      return false;
    }
  }, [client]);

  const openProject = useCallback(async () => {
    if (client === null) return false;
    setState((current) => ({ ...current, actionStatus: "selecting", error: null }));
    try {
      const projectPath = await client.pickOpenPath();
      if (projectPath === null) {
        setState((current) => ({ ...current, actionStatus: "idle" }));
        return false;
      }
      setState((current) => ({ ...current, actionStatus: "opening" }));
      const project = await client.open(projectPath);
      setState({ actionStatus: "idle", error: null, project, status: "active" });
      return true;
    } catch (error: unknown) {
      setState(errorState(error));
      return false;
    }
  }, [client]);

  const closeProject = useCallback(async () => {
    if (client === null) return;
    setState((current) => ({ ...current, actionStatus: "closing", error: null }));
    try {
      await client.close();
      setState({ actionStatus: "idle", error: null, project: null, status: "empty" });
    } catch (error: unknown) {
      setState(errorState(error));
    }
  }, [client]);

  const upgradeProject = useCallback(async () => {
    if (client === null) return false;
    setState((current) => ({
      ...current,
      actionStatus: "upgrading",
      error: null,
    }));
    try {
      const project = await client.upgrade();
      setState(upgradeSucceeded(project));
      return true;
    } catch (error: unknown) {
      setState((current) => upgradeFailed(current, error));
      return false;
    }
  }, [client]);

  const dismissError = useCallback(() => {
    setState((current) => ({
      actionStatus: "idle",
      error: null,
      project: current.project,
      status: current.project === null ? "empty" : "active",
    }));
  }, []);

  return {
    ...state,
    closeProject,
    createProject,
    dismissError,
    openProject,
    retry: loadCurrent,
    upgradeProject,
  };
}

export function upgradeSucceeded(project: ProjectDescriptor): ProjectLifecycleState {
  return {
    actionStatus: "idle",
    error: null,
    project,
    status: "active",
  };
}

export function upgradeFailed(
  current: ProjectLifecycleState,
  error: unknown,
): ProjectLifecycleState {
  const envelope = desktopErrorFrom(error);
  if (envelope.code === "PROJECT_CORRUPTED") {
    return {
      actionStatus: "idle",
      error: envelope,
      project: null,
      status: "error",
    };
  }
  return {
    actionStatus: "idle",
    error: envelope,
    project: current.project,
    status: current.project === null ? "error" : "active",
  };
}

function initialProjectState(
  client: ProjectClient | null,
  startupOverride?: StartupStateOverride,
): ProjectLifecycleState {
  if (startupOverride === "loading") return { actionStatus: "idle", error: null, project: null, status: "loading" };
  if (startupOverride === "error") return errorState(new Error("startup override"));
  if (startupOverride === "ready") return { actionStatus: "idle", error: null, project: null, status: "empty" };
  return client === null ? permissionState() : { actionStatus: "idle", error: null, project: null, status: "loading" };
}

function permissionState(): ProjectLifecycleState {
  return {
    actionStatus: "idle",
    error: {
      code: "PERMISSION_DENIED",
      correlation_id: "00000000-0000-7000-8000-000000000000",
      detail: "native desktop project runtime is unavailable",
      field_errors: [],
      message: "Akses proyek hanya tersedia di aplikasi desktop Teratai.",
      remediation: "Jalankan Teratai melalui desktop:dev atau executable native.",
      retriable: false,
    },
    project: null,
    status: "permission",
  };
}

function errorState(error: unknown): ProjectLifecycleState {
  const envelope = desktopErrorFrom(error);
  return {
    actionStatus: "idle",
    error: envelope,
    project: null,
    status: envelope.code === "PERMISSION_DENIED" ? "permission" : "error",
  };
}

function desktopErrorFrom(error: unknown): DesktopError {
  return error instanceof ProjectClientError
    ? error.envelope
    : {
        code: "OPERATION_FAILED",
        correlation_id: "00000000-0000-7000-8000-000000000000",
        detail: "desktop project lifecycle failed unexpectedly",
        field_errors: [],
        message: "Operasi proyek tidak dapat diselesaikan.",
        remediation: "Coba kembali. Jika masalah berulang, mulai ulang Teratai.",
        retriable: true,
      } satisfies DesktopError;
}
