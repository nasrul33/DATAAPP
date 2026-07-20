import { AppShell } from "./app-shell";

export type StartupState = "error" | "loading" | "ready";

export interface AppProps {
  readonly onRetry?: (() => void) | undefined;
  readonly startupState?: StartupState;
}

export function App({ onRetry, startupState = "ready" }: AppProps) {
  return <AppShell onRetry={onRetry} startupState={startupState} />;
}
