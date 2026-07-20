import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { App } from "../src/app/app";
import { startDesktopTrace } from "../src/runtime-logging";

describe("desktop application shell", () => {
  it("renders an accessible empty dashboard without exposing planned features", () => {
    const markup = renderToStaticMarkup(<App />);

    expect(markup).toContain("Lewati ke konten utama");
    expect(markup).toContain("Belum ada proyek analitik");
    expect(markup).toContain("Data sumber tidak pernah diubah langsung");
    expect(markup).toContain('aria-current="page"');
    expect(markup).toContain("Belum diaktifkan");
  });

  it("renders an explicit startup loading state", () => {
    const markup = renderToStaticMarkup(<App startupState="loading" />);

    expect(markup).toContain('aria-busy="true"');
    expect(markup).toContain("Menyiapkan ruang kerja");
  });

  it("renders a recoverable startup error state", () => {
    const onRetry = vi.fn();
    const markup = renderToStaticMarkup(<App onRetry={onRetry} startupState="error" />);

    expect(markup).toContain("Ruang kerja gagal disiapkan");
    expect(markup).toContain("Coba lagi");
    expect(onRetry).not.toHaveBeenCalled();
  });
});

describe("desktop runtime logging", () => {
  it("emits a safe canonical startup event with a UUID v7", () => {
    const events: unknown[] = [];
    const correlationId = startDesktopTrace((event) => events.push(event));

    expect(correlationId).toMatch(/^[0-9a-f-]{36}$/);
    expect(events).toEqual([
      expect.objectContaining({
        component: "desktop-shell",
        correlation_id: correlationId,
        event: "desktop.startup",
        layer: "desktop",
        sequence: 1,
      }),
    ]);
  });
});
