import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { execPath } from "node:process";
import test from "node:test";

const generatedContracts = [
  ["contract-metadata", "contract_metadata", "ContractMetadata"],
  ["correlation-request", "correlation_request", "CorrelationRequest"],
  ["desktop-error", "desktop_error", "DesktopError"],
  ["engine-error", "engine_error", "EngineError"],
  ["engine-handshake-request", "engine_handshake_request", "EngineHandshakeRequest"],
  ["engine-handshake-response", "engine_handshake_response", "EngineHandshakeResponse"],
  ["project-create-request", "project_create_request", "ProjectCreateRequest"],
  ["project-descriptor", "project_descriptor", "ProjectDescriptor"],
  ["project-manifest", "project_manifest", "ProjectManifest"],
  ["project-open-request", "project_open_request", "ProjectOpenRequest"],
  ["runtime-log-event", "runtime_log_event", "RuntimeLogEvent"],
  ["job-descriptor", "job_descriptor", "JobDescriptor"],
  ["job-enqueue-request", "job_enqueue_request", "JobEnqueueRequest"],
  ["job-failure-request", "job_failure_request", "JobFailureRequest"],
  ["job-get-request", "job_get_request", "JobGetRequest"],
  ["job-lifecycle-event", "job_lifecycle_event", "JobLifecycleEvent"],
  ["job-list-request", "job_list_request", "JobListRequest"],
  ["job-list-response", "job_list_response", "JobListResponse"],
  ["job-progress-update-request", "job_progress_update_request", "JobProgressUpdateRequest"],
  ["job-transition-request", "job_transition_request", "JobTransitionRequest"],
];

test("canonical schemas generate deterministically into three languages", async () => {
  const result = spawnSync(execPath, ["scripts/contracts/generate.mjs", "--check"], {
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr || result.stdout);
  const generatedPaths = generatedContracts.flatMap(([typescriptName, portableName]) => [
    `packages/contracts/src/generated/${typescriptName}.ts`,
    `engine/teratai_engine/generated/${portableName}.py`,
    `packages/contracts/rust/src/generated/${portableName}.rs`,
  ]);
  await Promise.all(generatedPaths.map((path) => access(path)));

  const generatedSources = await Promise.all(
    generatedPaths.map((path) => readFile(path, "utf8")),
  );
  for (const [index, source] of generatedSources.entries()) {
    assert.match(source, /Schema SHA-256: [0-9a-f]{64}/);
    const contract = generatedContracts[Math.floor(index / 3)];
    assert.ok(contract);
    assert.match(source, new RegExp(contract[2]));
  }
});

test("job contract fixtures are valid JSON documents", async () => {
  const fixtureNames = [
    "job-descriptor",
    "job-enqueue-request",
    "job-transition-request",
    "job-progress-update-request",
    "job-failure-request",
  ];

  const fixtures = await Promise.all(
    fixtureNames.map(async (fixtureName) => {
      const source = await readFile(`packages/contracts/fixtures/${fixtureName}.valid.json`, "utf8");
      return JSON.parse(source);
    }),
  );

  for (const fixture of fixtures) {
    assert.equal(typeof fixture, "object");
    assert.notEqual(fixture, null);
  }
});
