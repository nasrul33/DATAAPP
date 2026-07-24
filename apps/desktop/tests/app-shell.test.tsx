import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { DesktopError, ProjectDescriptor } from "@teratai/contracts";

import { App } from "../src/app/app";
import { AppShell } from "../src/app/app-shell";
import { ProjectDialog } from "../src/project/project-dialog";
import {
  canUpgradeProject,
  getNextFocusIndex,
  projectNameMatchesExactly,
  ProjectUpgradeDialog,
  ProjectUpgradePanel,
} from "../src/project/project-upgrade-panel";
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

  it("renders the actionable upgrade flow for schema-1 projects", () => {
    const markup = renderToStaticMarkup(
      <AppShell
        lifecycle={lifecycle({
          project: { ...descriptor, metadata_schema_version: 1 },
          status: "active",
        })}
      />,
    );

    expect(markup).toContain("Upgrade proyek diperlukan");
    expect(markup).toContain("Upgrade proyek");
    expect(markup).toContain("Metadata schema versi 1");
    expect(markup).not.toContain("tidak di-upgrade otomatis");
    expect(markup).not.toContain("Belum ada pekerjaan");
  });

  it("keeps dashboard controls disabled while a schema-1 upgrade is running", () => {
    const markup = renderToStaticMarkup(
      <AppShell
        lifecycle={lifecycle({
          actionStatus: "upgrading",
          project: { ...descriptor, metadata_schema_version: 1 },
          status: "active",
        })}
      />,
    );

    expect(markup).toContain("Meng-upgrade");
    const closeProjectButton = /<button\b[^>]*>(?:(?!<\/button>)[\s\S])*Tutup proyek<\/button>/.exec(markup)?.[0];
    expect(closeProjectButton).toContain('disabled=""');
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

describe("desktop project upgrade confirmation surface", () => {
  const schemaOneDescriptor = {
    ...descriptor,
    metadata_schema_version: 1,
  } satisfies ProjectDescriptor;

  it("requires the project name to match exactly without normalization", () => {
    expect(projectNameMatchesExactly("Audit 2026", "Audit 2026")).toBe(true);
    expect(projectNameMatchesExactly(" audit 2026", "Audit 2026")).toBe(false);
    expect(projectNameMatchesExactly("Audit 2026 ", "Audit 2026")).toBe(false);
    expect(projectNameMatchesExactly("audit 2026", "Audit 2026")).toBe(false);
    expect(projectNameMatchesExactly("Cafe\u0301", "Café")).toBe(false);
  });

  it("renders the schema-1 upgrade panel without path authority", () => {
    const markup = renderToStaticMarkup(
      <ProjectUpgradePanel
        actionStatus="idle"
        error={null}
        onDismissError={() => undefined}
        onUpgrade={() => Promise.resolve(true)}
        project={schemaOneDescriptor}
      />,
    );

    expect(markup).toContain("Metadata schema versi 1");
    expect(markup).toContain("memerlukan versi 2");
    expect(markup).toContain("Dataset sumber tetap tidak berubah");
    expect(markup).toContain("Tidak tersedia downgrade");
    expect(markup).toContain("Upgrade proyek");
    expect(markup).not.toContain(schemaOneDescriptor.project_path);
  });

  it("disables upgrade submission until the raw project name matches exactly", () => {
    const mismatched = renderToStaticMarkup(
      <ProjectUpgradeDialog
        confirmationValue="Audit Belanja 2026 "
        error={null}
        onCancel={() => undefined}
        onConfirmationChange={() => undefined}
        onDismissError={() => undefined}
        onRetry={() => undefined}
        onSubmit={() => undefined}
        pending={false}
        projectName={schemaOneDescriptor.name}
      />,
    );
    const matched = renderToStaticMarkup(
      <ProjectUpgradeDialog
        confirmationValue={schemaOneDescriptor.name}
        error={null}
        onCancel={() => undefined}
        onConfirmationChange={() => undefined}
        onDismissError={() => undefined}
        onRetry={() => undefined}
        onSubmit={() => undefined}
        pending={false}
        projectName={schemaOneDescriptor.name}
      />,
    );

    expect(mismatched).toContain('disabled="" type="submit"');
    expect(matched).toContain('type="submit"');
    expect(matched).not.toContain('disabled="" type="submit"');
  });

  it("renders a non-dismissible pending upgrade dialog", () => {
    const markup = renderToStaticMarkup(
      <ProjectUpgradeDialog
        confirmationValue={schemaOneDescriptor.name}
        error={null}
        onCancel={() => undefined}
        onConfirmationChange={() => undefined}
        onDismissError={() => undefined}
        onRetry={() => undefined}
        onSubmit={() => undefined}
        pending
        projectName={schemaOneDescriptor.name}
      />,
    );

    expect(markup).toContain('role="alertdialog"');
    expect(markup).toContain('aria-modal="true"');
    expect(markup).toContain('aria-busy="true"');
    expect(markup).toContain("Meng-upgrade proyek");
    expect(markup).toContain('autoComplete="off"');
    expect(markup).toContain('spellCheck="false"');
    expect(markup.match(/disabled=""/g)).toHaveLength(4);
  });

  it("keeps safe retriable upgrade feedback inside an open dialog without clearing confirmation", () => {
    const error = desktopError({
      correlation_id: "00000000-0000-7000-8000-000000000778",
      detail: "D:\\confidential\\metadata.sqlite",
      field_errors: ["Metadata schema belum siap."],
      message: "Upgrade proyek belum dapat diselesaikan.",
      remediation: "Coba kembali setelah memeriksa ruang penyimpanan.",
      retriable: true,
    });
    const markup = renderToStaticMarkup(
      <ProjectUpgradeDialog
        confirmationValue={schemaOneDescriptor.name}
        error={error}
        onCancel={() => undefined}
        onConfirmationChange={() => undefined}
        onDismissError={() => undefined}
        onRetry={() => undefined}
        onSubmit={() => undefined}
        pending={false}
        projectName={schemaOneDescriptor.name}
      />,
    );

    expect(markup).toContain('role="alertdialog"');
    expect(markup).toContain(`value="${schemaOneDescriptor.name}"`);
    expect(markup).toContain(error.message);
    expect(markup).toContain(error.remediation);
    expect(markup).toContain(error.field_errors[0]);
    expect(markup).toContain(error.correlation_id);
    expect(markup).toContain("Tutup pesan");
    expect(markup).toContain("Coba lagi");
    expect(markup).not.toContain(error.detail);
  });

  it("cycles focus to the opposite dialog boundary on Tab", () => {
    expect(getNextFocusIndex(0, 4, true)).toBe(3);
    expect(getNextFocusIndex(3, 4, false)).toBe(0);
    expect(getNextFocusIndex(1, 4, false)).toBeNull();
    expect(getNextFocusIndex(0, 0, false)).toBeNull();
  });

  it("renders safe retriable error details with retry controls", () => {
    const error = desktopError({
      correlation_id: "00000000-0000-7000-8000-000000000777",
      detail: "D:\\confidential\\metadata.sqlite",
      field_errors: ["Metadata schema belum siap."],
      message: "Upgrade proyek belum dapat diselesaikan.",
      remediation: "Coba kembali setelah memeriksa ruang penyimpanan.",
      retriable: true,
    });
    const markup = renderToStaticMarkup(
      <ProjectUpgradePanel
        actionStatus="idle"
        error={error}
        onDismissError={() => undefined}
        onUpgrade={() => Promise.resolve(false)}
        project={schemaOneDescriptor}
      />,
    );

    expect(markup).toContain(error.message);
    expect(markup).toContain(error.remediation);
    expect(markup).toContain(error.field_errors[0]);
    expect(markup).toContain(error.correlation_id);
    expect(markup).toContain("Tutup pesan");
    expect(markup).toContain("Coba lagi");
    expect(markup).not.toContain(error.detail);
  });

  it("renders a non-destructive recovery error without retry controls", () => {
    const error = desktopError({
      code: "PROJECT_CORRUPTED",
      message: "Proyek memerlukan pemulihan sebelum dapat dibuka.",
      remediation: "Jangan hapus berkas proyek.",
      retriable: false,
    });
    const markup = renderToStaticMarkup(
      <ProjectUpgradePanel
        actionStatus="idle"
        error={error}
        onDismissError={() => undefined}
        onUpgrade={() => Promise.resolve(false)}
        project={schemaOneDescriptor}
      />,
    );

    expect(markup).toContain(error.remediation);
    expect(markup).not.toContain("Coba lagi");
    expect(markup).not.toContain("Hapus proyek");
    expect(markup).not.toContain("Perbaiki proyek");
    expect(markup).not.toContain("Downgrade proyek");
  });

  it("blocks every upgrade path for a closed recovery panel", () => {
    const error = desktopError({
      code: "PROJECT_CORRUPTED",
      message: "Proyek memerlukan pemulihan sebelum dapat dibuka.",
      retriable: false,
    });
    const markup = renderToStaticMarkup(
      <ProjectUpgradePanel
        actionStatus="idle"
        error={error}
        onDismissError={() => undefined}
        onUpgrade={() => Promise.resolve(false)}
        project={schemaOneDescriptor}
      />,
    );

    expect(canUpgradeProject("idle", error)).toBe(false);
    expect(markup).toContain('disabled="" type="button"');
    expect(markup).not.toContain("Coba lagi");
  });

  it("blocks exact-name upgrade submission in an open recovery dialog", () => {
    const error = desktopError({
      code: "PROJECT_CORRUPTED",
      message: "Proyek memerlukan pemulihan sebelum dapat dibuka.",
      retriable: false,
    });
    const markup = renderToStaticMarkup(
      <ProjectUpgradeDialog
        confirmationValue={schemaOneDescriptor.name}
        error={error}
        onCancel={() => undefined}
        onConfirmationChange={() => undefined}
        onDismissError={() => undefined}
        onRetry={() => undefined}
        onSubmit={() => undefined}
        pending={false}
        projectName={schemaOneDescriptor.name}
      />,
    );

    expect(markup).toContain('disabled="" type="submit"');
    expect(markup).not.toContain("Coba lagi");
    expect(markup).not.toContain("Downgrade proyek");
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
