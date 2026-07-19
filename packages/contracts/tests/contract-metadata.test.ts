import { describe, expect, it } from "vitest";

import type { ContractMetadata } from "../src/index";

describe("generated contract metadata", () => {
  it("round-trips the canonical fixture shape", () => {
    const metadata = {
      generator_revision: 1,
      protocol_version: "1.0",
      schema_name: "contract-metadata",
      schema_version: "1.0.0",
    } satisfies ContractMetadata;

    expect(JSON.parse(JSON.stringify(metadata))).toEqual(metadata);
  });
});
