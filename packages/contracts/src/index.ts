export const ENGINE_PROTOCOL_VERSION = "1.0" as const;

export type { ContractMetadata } from "./generated/contract-metadata";
export type { CorrelationRequest } from "./generated/correlation-request";
export type { DesktopError } from "./generated/desktop-error";
export type { EngineError } from "./generated/engine-error";
export type { EngineHandshakeRequest } from "./generated/engine-handshake-request";
export type { EngineHandshakeResponse } from "./generated/engine-handshake-response";
export type { JobDescriptor } from "./generated/job-descriptor";
export type { JobEnqueueRequest } from "./generated/job-enqueue-request";
export type { JobFailureRequest } from "./generated/job-failure-request";
export type { JobGetRequest } from "./generated/job-get-request";
export type { JobLifecycleEvent } from "./generated/job-lifecycle-event";
export type { JobListRequest } from "./generated/job-list-request";
export type { JobListResponse } from "./generated/job-list-response";
export type { JobProgressUpdateRequest } from "./generated/job-progress-update-request";
export type { JobTransitionRequest } from "./generated/job-transition-request";
export type { ProjectCreateRequest } from "./generated/project-create-request";
export type { ProjectDescriptor } from "./generated/project-descriptor";
export type { ProjectManifest } from "./generated/project-manifest";
export type { ProjectOpenRequest } from "./generated/project-open-request";
export type { RuntimeLogEvent } from "./generated/runtime-log-event";
export {
  createCorrelationId,
  createRuntimeLogEvent,
  type RuntimeLogInput,
} from "./runtime-logging";
