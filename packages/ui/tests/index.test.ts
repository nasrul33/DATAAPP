import { describe, expect, it } from "vitest";

import { UI_PACKAGE_STATUS } from "../src/index";

describe("UI workspace boundary", () => {
  it("is initialized without product components", () => {
    expect(UI_PACKAGE_STATUS).toBe("foundation");
  });
});
