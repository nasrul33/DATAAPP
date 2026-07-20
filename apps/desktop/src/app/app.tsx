import { useMemo } from "react";

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
  readonly projectClient?: ProjectClient | null;
  readonly startupState?: StartupState;
}

export function App({ projectClient, startupState }: AppProps) {
  const runtimeClient = useMemo(() => {
    if (projectClient !== undefined) return projectClient;
    return projectRuntimeAvailable() ? tauriProjectClient : null;
  }, [projectClient]);
  const lifecycle = useProjectLifecycle(runtimeClient, startupState);

  return <AppShell lifecycle={lifecycle} />;
}
