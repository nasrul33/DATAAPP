import { describe, expect, it } from "vitest";

import { ENGINE_PROTOCOL_VERSION } from "../src/index";

describe("contracts workspace boundary", () => {
  it("exposes the initial protocol version", () => {
    expect(ENGINE_PROTOCOL_VERSION).toBe("1.0");
  });
});
