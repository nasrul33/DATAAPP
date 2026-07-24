import { useMemo } from "react";

import {
  tauriJobClient,
  type JobClient,
} from "../job/job-client";
import {
  projectRuntimeAvailable,
  tauriProjectClient,
  type ProjectClient,
} from "../project/project-client";
import {
  useProjectLifecycle,
  type StartupStateOverride,
} from "../project/use-project-lifecycle";
import { AppShell } from "./app-shell";

export type StartupState = StartupStateOverride;

export interface AppProps {
  readonly jobClient?: JobClient | null;
  readonly projectClient?: ProjectClient | null;
  readonly startupState?: StartupState;
}

export function App({ jobClient, projectClient, startupState }: AppProps) {
  const runtimeClient = useMemo(() => {
    if (projectClient !== undefined) return projectClient;
    return projectRuntimeAvailable() ? tauriProjectClient : null;
  }, [projectClient]);
  const runtimeJobClient = useMemo(() => {
    if (jobClient !== undefined) return jobClient;
    return projectRuntimeAvailable() ? tauriJobClient : null;
  }, [jobClient]);
  const lifecycle = useProjectLifecycle(runtimeClient, startupState);

  return <AppShell jobClient={runtimeJobClient} lifecycle={lifecycle} />;
}
