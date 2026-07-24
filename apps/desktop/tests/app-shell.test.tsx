import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { DesktopError, ProjectDescriptor } from "@teratai/contracts";

import { App } from "../src/app/app";
import { AppShell } from "../src/app/app-shell";
import { ProjectDialog } from "../src/project/project-dialog";
import {
  parseDesktopError,
  parseProjectDescriptor,
  ProjectClientError,
  suggestedProjectFileName,
} from "../src/project/project-client";
import {
  upgradeFailed,
  upgradeSucceeded,
  type ProjectLifecycle,
  type ProjectLifecycleState,
} from "../src/project/use-project-lifecycle";
import { startDesktopTrace } from "../src/runtime-logging";

const descriptor = {
  created_at: "2026-07-20T12:00:00Z",
  metadata_schema_version: 2,
  name: "Audit Belanja 2026",
  project_id: "00000000-0000-7000-8000-000000000110",
  project_path: "D:\\Projects\\Audit Belanja 2026.teratai",
  schema_version: "1.0.0",
} satisfies ProjectDescriptor;

describe("desktop project lifecycle UI", () => {
  it("renders an actionable accessible empty dashboard", () => {
    const markup = renderToStaticMarkup(<App projectClient={null} startupState="ready" />);

    expect(markup).toContain("Lewati ke konten utama");
    expect(markup).toContain("Belum ada proyek analitik");
    expect(markup).toContain("Buat proyek");
    expect(markup).toContain("Buka proyek");
    expect(markup).toContain("Data sumber tidak pernah diubah langsung");
    expect(markup).toContain('aria-current="page"');
  });

  it("renders startup loading and browser permission states explicitly", () => {
    const loading = renderToStaticMarkup(<App projectClient={null} startupState="loading" />);
    const permission = renderToStaticMarkup(<App projectClient={null} />);

    expect(loading).toContain('aria-busy="true"');
    expect(loading).toContain("Memvalidasi ruang kerja lokal");
    expect(permission).toContain("Akses lokasi diperlukan");
    expect(permission).toContain("aplikasi desktop Teratai");
  });

  it("renders a verified active project without enabling ingestion", () => {
    const markup = renderToStaticMarkup(
      <AppShell lifecycle={lifecycle({ project: descriptor, status: "active" })} />,
    );

    expect(markup).toContain("Proyek terverifikasi");
    expect(markup).toContain("Audit Belanja 2026");
    expect(markup).toContain("Project storage siap");
    expect(markup).toContain("Job Center hanya tersedia di aplikasi desktop");
    expect(markup).toContain("Tutup proyek");
    expect(markup).toContain("Import dataset tetap nonaktif");
  });

  it("renders a non-mutating upgrade state for schema-1 projects", () => {
    const markup = renderToStaticMarkup(
      <AppShell
        lifecycle={lifecycle({
          project: { ...descriptor, metadata_schema_version: 1 },
          status: "active",
        })}
      />,
    );

    expect(markup).toContain("Upgrade proyek diperlukan");
    expect(markup).toContain("tidak di-upgrade otomatis");
  });

  it("renders recovery remediation without a destructive action", () => {
    const error = desktopError({
      code: "PROJECT_CORRUPTED",
      message: "Proyek memerlukan pemulihan sebelum dapat dibuka.",
      remediation: "Jangan hapus berkas proyek.",
      retriable: false,
    });
    const markup = renderToStaticMarkup(
      <AppShell lifecycle={lifecycle({ error, status: "error" })} />,
    );

    expect(markup).toContain("Proyek memerlukan pemulihan");
    expect(markup).toContain("Jangan hapus berkas proyek");
    expect(markup).not.toContain("Hapus proyek");
    expect(markup).not.toContain("Coba lagi");
  });

  it("renders a labeled creation dialog with disabled empty submit", () => {
    const markup = renderToStaticMarkup(
      <ProjectDialog actionStatus="idle" onClose={() => undefined} onCreate={() => Promise.resolve(true)} />,
    );

    expect(markup).toContain('role="dialog"');
    expect(markup).toContain('aria-modal="true"');
    expect(markup).toContain("Nama proyek");
    expect(markup).toContain("Pilih lokasi");
    expect(markup).toContain("disabled");
  });
});

describe("desktop project boundary helpers", () => {
  it("sanitizes Windows project filenames deterministically", () => {
    expect(suggestedProjectFileName("Audit: Belanja / 2026"))
      .toBe("Audit- Belanja - 2026.teratai");
    expect(suggestedProjectFileName("CON")).toBe("Proyek Teratai.teratai");
  });

  it("validates native descriptor and error envelopes", () => {
    expect(parseProjectDescriptor(descriptor)).toEqual(descriptor);
    expect(() => parseProjectDescriptor({ name: "incomplete" })).toThrow();
    expect(parseDesktopError({ unexpected: true }).code).toBe("OPERATION_FAILED");
  });
});

describe("desktop project upgrade lifecycle transitions", () => {
  const schemaOneDescriptor = {
    ...descriptor,
    metadata_schema_version: 1,
  } satisfies ProjectDescriptor;
  const schemaTwoDescriptor = {
    ...descriptor,
    metadata_schema_version: 2,
  } satisfies ProjectDescriptor;
  const schemaOneState: ProjectLifecycleState = {
    actionStatus: "upgrading",
    error: null,
    project: schemaOneDescriptor,
    status: "active",
  };

  it("publishes the validated schema-2 descriptor only after upgrade succeeds", () => {
    expect(upgradeSucceeded(schemaTwoDescriptor)).toEqual({
      actionStatus: "idle",
      error: null,
      project: schemaTwoDescriptor,
      status: "active",
    });
  });

  it("preserves the active schema-1 descriptor when upgrade failure is retriable", () => {
    const retriableError = desktopError({
      code: "OPERATION_FAILED",
      retriable: true,
    });

    expect(upgradeFailed(schemaOneState, new ProjectClientError(retriableError))).toEqual({
      actionStatus: "idle",
      error: retriableError,
      project: schemaOneDescriptor,
      status: "active",
    });
  });

  it("escalates project corruption to the full recovery error state", () => {
    const recoveryError = desktopError({
      code: "PROJECT_CORRUPTED",
      message: "Proyek memerlukan pemulihan sebelum dapat dibuka.",
      remediation: "Jangan hapus berkas proyek.",
      retriable: false,
    });

    expect(upgradeFailed(schemaOneState, new ProjectClientError(recoveryError))).toEqual({
      actionStatus: "idle",
      error: recoveryError,
      project: null,
      status: "error",
    });
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

function lifecycle(overrides: Partial<ProjectLifecycle>): ProjectLifecycle {
  return {
    actionStatus: "idle",
    closeProject: () => Promise.resolve(),
    createProject: () => Promise.resolve(false),
    dismissError: () => undefined,
    error: null,
    openProject: () => Promise.resolve(false),
    project: null,
    retry: () => Promise.resolve(),
    status: "empty",
    upgradeProject: () => Promise.resolve(false),
    ...overrides,
  };
}

function desktopError(overrides: Partial<DesktopError>): DesktopError {
  return {
    code: "OPERATION_FAILED",
    correlation_id: "00000000-0000-7000-8000-000000000111",
    detail: "project command failed",
    field_errors: [],
    message: "Operasi proyek gagal.",
    remediation: "Coba kembali.",
    retriable: true,
    ...overrides,
  };
}
