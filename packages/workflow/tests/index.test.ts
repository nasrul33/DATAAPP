import { describe, expect, it } from "vitest";

import { WORKFLOW_PACKAGE_STATUS } from "../src/index";

describe("workflow workspace boundary", () => {
  it("is initialized without product behavior", () => {
    expect(WORKFLOW_PACKAGE_STATUS).toBe("foundation");
  });
});
