import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { execPath } from "node:process";
import test from "node:test";

const generatedPaths = [
  "packages/contracts/src/generated/contract-metadata.ts",
  "engine/teratai_engine/generated/contract_metadata.py",
  "crates/app-core/src/generated/contract_metadata.rs",
];

test("T-0004 generates one canonical schema into three languages", async () => {
  const result = spawnSync(execPath, ["scripts/contracts/generate.mjs", "--check"], {
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr || result.stdout);
  await Promise.all(generatedPaths.map((path) => access(path)));

  const generatedSources = await Promise.all(
    generatedPaths.map((path) => readFile(path, "utf8")),
  );
  for (const source of generatedSources) {
    assert.match(source, /Schema SHA-256: [0-9a-f]{64}/);
    assert.match(source, /ContractMetadata/);
  }
});
