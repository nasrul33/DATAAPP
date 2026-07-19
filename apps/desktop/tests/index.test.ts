import { describe, expect, it } from "vitest";

import { desktopTarget } from "../src/index";

describe("desktop workspace boundary", () => {
  it("pins the Tauri major target", () => {
    expect(desktopTarget).toEqual({ shell: "tauri", version: "2.x" });
  });
});
