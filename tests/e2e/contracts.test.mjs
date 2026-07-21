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
