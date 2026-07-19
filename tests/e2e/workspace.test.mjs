import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";

const requiredPaths = [
  "apps/desktop",
  "crates/app-core",
  "crates/filesystem",
  "crates/engine-host",
  "crates/secure-store",
  "engine/teratai_engine",
  "packages/contracts",
  "packages/ui",
  "packages/workflow",
  "packages/config",
  "tests/e2e",
  "tests/fixtures",
  "tests/golden",
];

test("all T-0001 workspace paths exist", async () => {
  await Promise.all(requiredPaths.map((path) => access(path)));
  assert.equal(requiredPaths.length, 13);
});

test("T-0003 CI runs every standard root gate with pinned actions", async () => {
  const workflow = await readFile(".github/workflows/quality.yml", "utf8");
  const requiredCommands = [
    "pnpm install --frozen-lockfile",
    "uv sync --frozen",
    "cargo fetch --locked",
    "pnpm lint",
    "pnpm typecheck",
    "pnpm test",
    "pnpm build",
  ];

  for (const command of requiredCommands) {
    assert.match(workflow, new RegExp(`run: ${command.replaceAll(" ", "\\s+")}`));
  }

  const actionReferences = workflow.match(/uses:\s+\S+/g) ?? [];
  assert.equal(actionReferences.length, 4);
  for (const reference of actionReferences) {
    assert.match(reference, /@[0-9a-f]{40}$/);
  }

  assert.match(workflow, /permissions:\s*\r?\n\s+contents: read/);
  assert.match(workflow, /persist-credentials: false/);
  assert.doesNotMatch(workflow, /pull_request_target|contents:\s*write/);
});
