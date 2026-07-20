import {
  createCorrelationId,
  createRuntimeLogEvent,
  type RuntimeLogEvent,
} from "@teratai/contracts";

export type RuntimeLogSink = (event: RuntimeLogEvent) => void;

/** Start one desktop trace without including browser state, project data, or paths. */
export function startDesktopTrace(sink: RuntimeLogSink): string {
  const correlationId = createCorrelationId();
  sink(createRuntimeLogEvent({
    component: "desktop-shell",
    correlationId,
    event: "desktop.startup",
    layer: "desktop",
    level: "INFO",
    message: "Desktop shell dimulai.",
    sequence: 1,
  }));
  return correlationId;
}
