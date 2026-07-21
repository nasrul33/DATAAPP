import { useEffect, useState } from "react";
import {
  Activity,
  BarChart3,
  BriefcaseBusiness,
  ChevronDown,
  CircleAlert,
  Database,
  FileCheck2,
  FileSearch,
  Flower2,
  FolderOpen,
  FolderPlus,
  HardDrive,
  LayoutDashboard,
  LoaderCircle,
  LockKeyhole,
  PanelLeft,
  RefreshCw,
  Settings,
  Share2,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  Workflow,
  X,
} from "lucide-react";

import type { DesktopError, ProjectDescriptor } from "@teratai/contracts";

import { ProjectDialog } from "../project/project-dialog";
import type { ProjectLifecycle } from "../project/use-project-lifecycle";

interface AppShellProps {
  readonly lifecycle: ProjectLifecycle;
}

const navigation = [
  { icon: LayoutDashboard, label: "Dashboard", status: "active" },
  { icon: BriefcaseBusiness, label: "Proyek", status: "planned" },
  { icon: Database, label: "Dataset", status: "planned" },
  { icon: Workflow, label: "Workflow", status: "planned" },
  { icon: FileSearch, label: "Temuan", status: "planned" },
  { icon: Share2, label: "Ekspor", status: "planned" },
] as const;

export function AppShell({ lifecycle }: AppShellProps) {
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const projectActive = lifecycle.project !== null && lifecycle.status === "active";

  useEffect(() => {
    if (lifecycle.status !== "empty") setCreateDialogOpen(false);
  }, [lifecycle.status]);

  return (
    <div className="min-h-screen bg-stone-100 text-stone-950">
      <a className="skip-link" href="#main-content">Lewati ke konten utama</a>
      <div className="grid min-h-screen lg:grid-cols-[17rem_1fr]">
        <Sidebar />
        <div className="min-w-0">
          <Header project={projectActive ? lifecycle.project : null} />
          <main className="mx-auto max-w-[96rem] p-5 md:p-8" id="main-content">
            {lifecycle.status === "loading" ? <LoadingDashboard /> : null}
            {lifecycle.status === "empty" ? (
              <EmptyDashboard
                actionStatus={lifecycle.actionStatus}
                onCreate={() => {
                  setCreateDialogOpen(true);
                }}
                onOpen={() => void lifecycle.openProject()}
              />
            ) : null}
            {lifecycle.status === "active" && lifecycle.project !== null ? (
              <ActiveProjectDashboard
                actionStatus={lifecycle.actionStatus}
                onClose={() => void lifecycle.closeProject()}
                project={lifecycle.project}
              />
            ) : null}
            {lifecycle.status === "error" && lifecycle.error !== null ? (
              <ProjectErrorDashboard
                error={lifecycle.error}
                onDismiss={lifecycle.dismissError}
                onRetry={() => void lifecycle.retry()}
              />
            ) : null}
            {lifecycle.status === "permission" && lifecycle.error !== null ? (
              <PermissionDashboard error={lifecycle.error} onDismiss={lifecycle.dismissError} />
            ) : null}
          </main>
        </div>
      </div>
      {createDialogOpen ? (
        <ProjectDialog
          actionStatus={lifecycle.actionStatus}
          onClose={() => {
            setCreateDialogOpen(false);
          }}
          onCreate={lifecycle.createProject}
        />
      ) : null}
    </div>
  );
}

function Sidebar() {
  return (
    <aside
      aria-label="Navigasi utama"
      className="flex flex-col border-r border-emerald-950/50 bg-[#08251d] px-4 py-5 text-emerald-50"
    >
      <div className="flex items-center gap-3 px-2">
        <span className="grid size-10 place-items-center rounded-xl bg-emerald-400 text-emerald-950 shadow-lg shadow-emerald-950/30" aria-hidden="true">
          <Flower2 size={22} strokeWidth={2.4} />
        </span>
        <div>
          <p className="text-sm font-semibold tracking-wide">Teratai</p>
          <p className="text-xs text-emerald-200/70">Analytics Desktop</p>
        </div>
      </div>
      <nav aria-label="Area aplikasi" className="mt-9">
        <p className="px-3 text-[0.68rem] font-bold uppercase tracking-[0.18em] text-emerald-200/45">Ruang kerja</p>
        <ul className="mt-3 space-y-1">
          {navigation.map(({ icon: Icon, label, status }) => (
            <li key={label}>
              <button
                aria-current={status === "active" ? "page" : undefined}
                aria-disabled={status === "planned"}
                className={`flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left text-sm font-medium transition ${status === "active" ? "bg-emerald-400 text-emerald-950 shadow-sm" : "text-emerald-100/65 disabled:cursor-not-allowed"}`}
                disabled={status === "planned"}
                type="button"
              >
                <Icon size={17} aria-hidden="true" />
                <span>{label}</span>
                {status === "planned" ? (
                  <span className="ml-auto rounded-full border border-emerald-200/15 px-2 py-0.5 text-[0.62rem] font-bold uppercase tracking-wide text-emerald-100/45">Segera</span>
                ) : null}
              </button>
            </li>
          ))}
        </ul>
      </nav>
      <div className="mt-auto space-y-3 pt-8">
        <div className="rounded-xl border border-emerald-200/10 bg-emerald-950/35 p-3">
          <div className="flex items-center gap-2 text-xs font-semibold text-emerald-100">
            <HardDrive size={15} aria-hidden="true" />
            Penyimpanan lokal
          </div>
          <p className="mt-1.5 text-xs leading-5 text-emerald-200/55">Data tetap berada di perangkat ini.</p>
        </div>
        <button className="flex w-full cursor-not-allowed items-center gap-3 rounded-lg px-3 py-2.5 text-sm font-medium text-emerald-100/45" disabled type="button">
          <Settings size={17} aria-hidden="true" />
          Pengaturan
        </button>
      </div>
    </aside>
  );
}

function Header({ project }: { readonly project: ProjectDescriptor | null }) {
  return (
    <header className="flex min-h-16 items-center justify-between border-b border-stone-200 bg-white/90 px-5 backdrop-blur md:px-8">
      <div className="flex min-w-0 items-center gap-3">
        <button aria-label="Buka navigasi" className="grid size-9 cursor-not-allowed place-items-center rounded-lg border border-stone-200 text-stone-400 lg:hidden" disabled type="button">
          <PanelLeft size={18} aria-hidden="true" />
        </button>
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold text-stone-900">{project?.name ?? "Ruang kerja lokal"}</p>
          <p className="truncate text-xs text-stone-500">{project === null ? "Belum ada proyek aktif" : "Proyek terverifikasi dan aktif"}</p>
        </div>
      </div>
      <button className="flex cursor-not-allowed items-center gap-2 rounded-lg border border-stone-200 bg-stone-50 px-3 py-2 text-xs font-semibold text-stone-500" disabled type="button">
        <span className="size-2 rounded-full bg-emerald-600" aria-hidden="true" />
        Mode lokal
        <ChevronDown size={14} aria-hidden="true" />
      </button>
    </header>
  );
}

function LoadingDashboard() {
  return (
    <section aria-busy="true" aria-label="Memuat ruang kerja" className="animate-pulse space-y-6">
      <div className="flex items-center gap-3 text-sm font-semibold text-emerald-800">
        <LoaderCircle className="animate-spin" size={18} aria-hidden="true" />
        Memvalidasi ruang kerja lokal…
      </div>
      <div className="h-20 max-w-2xl rounded-2xl bg-stone-200" />
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {Array.from({ length: 4 }, (_, index) => <div className="h-32 rounded-2xl border border-stone-200 bg-white" key={index} />)}
      </div>
      <div className="h-80 rounded-2xl border border-stone-200 bg-white" />
    </section>
  );
}

interface EmptyDashboardProps {
  readonly actionStatus: ProjectLifecycle["actionStatus"];
  readonly onCreate: () => void;
  readonly onOpen: () => void;
}

function EmptyDashboard({ actionStatus, onCreate, onOpen }: EmptyDashboardProps) {
  const pending = actionStatus !== "idle";
  const metrics = [
    { icon: BriefcaseBusiness, label: "Proyek aktif", value: "0" },
    { icon: Database, label: "Dataset", value: "0" },
    { icon: Workflow, label: "Workflow", value: "0" },
    { icon: FileSearch, label: "Temuan", value: "0" },
  ] as const;
  return (
    <div className="space-y-6">
      <section className="flex flex-col justify-between gap-5 sm:flex-row sm:items-end" aria-labelledby="dashboard-title">
        <div>
          <div className="flex items-center gap-2 text-xs font-bold uppercase tracking-[0.16em] text-emerald-800">
            <Sparkles size={14} aria-hidden="true" /> Analitik yang dapat dijelaskan
          </div>
          <h1 className="mt-2 text-3xl font-bold tracking-tight" id="dashboard-title">Dashboard</h1>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-stone-600">Buat ruang kerja baru atau buka proyek lokal yang sudah tervalidasi.</p>
        </div>
        <div className="flex flex-col gap-2 sm:flex-row">
          <button className="inline-flex items-center justify-center gap-2 rounded-lg border border-stone-300 bg-white px-4 py-2.5 text-sm font-semibold text-stone-700 shadow-sm hover:bg-stone-50 disabled:cursor-wait disabled:opacity-60" disabled={pending} onClick={onOpen} type="button">
            {actionStatus === "opening" || actionStatus === "selecting" ? <LoaderCircle className="animate-spin" size={17} aria-hidden="true" /> : <FolderOpen size={17} aria-hidden="true" />}
            Buka proyek
          </button>
          <button className="inline-flex items-center justify-center gap-2 rounded-lg bg-emerald-800 px-4 py-2.5 text-sm font-semibold text-white shadow-sm hover:bg-emerald-900 disabled:cursor-wait disabled:bg-emerald-700/60" disabled={pending} onClick={onCreate} type="button">
            <FolderPlus size={17} aria-hidden="true" /> Buat proyek
          </button>
        </div>
      </section>
      <section aria-label="Ringkasan ruang kerja" className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {metrics.map(({ icon: Icon, label, value }) => (
          <article className="rounded-2xl border border-stone-200 bg-white p-5 shadow-sm" key={label}>
            <div className="flex items-center justify-between">
              <span className="grid size-9 place-items-center rounded-lg bg-emerald-50 text-emerald-800" aria-hidden="true"><Icon size={18} /></span>
              <span className="rounded-full bg-stone-100 px-2 py-1 text-[0.65rem] font-bold uppercase tracking-wide text-stone-500">Kosong</span>
            </div>
            <p className="mt-5 text-2xl font-bold tabular-nums">{value}</p>
            <p className="mt-1 text-sm text-stone-600">{label}</p>
          </article>
        ))}
      </section>
      <DashboardGrid>
        <section aria-labelledby="empty-project-title" className="rounded-2xl border border-stone-200 bg-white p-6 shadow-sm">
          <div className="grid min-h-64 place-items-center rounded-xl border border-dashed border-stone-300 bg-stone-50/70 px-6 py-10 text-center">
            <div className="max-w-lg">
              <span className="mx-auto grid size-14 place-items-center rounded-2xl bg-emerald-100 text-emerald-900" aria-hidden="true"><BarChart3 size={27} /></span>
              <h2 className="mt-5 text-lg font-bold" id="empty-project-title">Belum ada proyek analitik</h2>
              <p className="mt-2 text-sm leading-6 text-stone-600">Proyek menyimpan dataset, workflow, hasil analisis, dan jejak audit secara lokal.</p>
              <div className="mt-5 inline-flex items-center gap-2 rounded-full border border-emerald-200 bg-emerald-50 px-3 py-1.5 text-xs font-semibold text-emerald-900">
                <ShieldCheck size={14} aria-hidden="true" /> Data sumber tidak pernah diubah langsung
              </div>
            </div>
          </div>
        </section>
        <SystemStatus workspaceStatus="Belum dipilih" />
      </DashboardGrid>
    </div>
  );
}

function ActiveProjectDashboard({ actionStatus, onClose, project }: {
  readonly actionStatus: ProjectLifecycle["actionStatus"];
  readonly onClose: () => void;
  readonly project: ProjectDescriptor;
}) {
  const createdAt = new Intl.DateTimeFormat("id-ID", {
    dateStyle: "long",
    timeStyle: "short",
    timeZone: "Asia/Jakarta",
  }).format(new Date(project.created_at));
  return (
    <div className="space-y-6">
      <section className="flex flex-col justify-between gap-5 sm:flex-row sm:items-end" aria-labelledby="active-project-title">
        <div>
          <div className="flex items-center gap-2 text-xs font-bold uppercase tracking-[0.16em] text-emerald-800"><FileCheck2 size={14} aria-hidden="true" /> Proyek terverifikasi</div>
          <h1 className="mt-2 text-3xl font-bold tracking-tight" id="active-project-title">{project.name}</h1>
          <p className="mt-2 max-w-3xl break-all text-sm leading-6 text-stone-600">{project.project_path}</p>
        </div>
        <button className="inline-flex items-center justify-center gap-2 rounded-lg border border-stone-300 bg-white px-4 py-2.5 text-sm font-semibold text-stone-700 shadow-sm hover:bg-stone-50 disabled:cursor-wait disabled:opacity-60" disabled={actionStatus !== "idle"} onClick={onClose} type="button">
          {actionStatus === "closing" ? <LoaderCircle className="animate-spin" size={16} aria-hidden="true" /> : <X size={16} aria-hidden="true" />}
          Tutup proyek
        </button>
      </section>
      <section aria-label="Identitas proyek" className="grid gap-4 md:grid-cols-3">
        <ProjectMetric label="Status" value="Terverifikasi" />
        <ProjectMetric label="Schema metadata" value={`Versi ${String(project.metadata_schema_version)}`} />
        <ProjectMetric label="Dibuat" value={createdAt} />
      </section>
      <DashboardGrid>
        <section aria-labelledby="project-ready-title" className="rounded-2xl border border-stone-200 bg-white p-6 shadow-sm">
          <div className="rounded-xl border border-emerald-200 bg-emerald-50/60 p-6">
            <div className="flex items-start gap-4">
              <span className="grid size-11 shrink-0 place-items-center rounded-xl bg-emerald-700 text-white" aria-hidden="true"><ShieldCheck size={21} /></span>
              <div>
                <h2 className="font-bold" id="project-ready-title">Project storage siap</h2>
                <p className="mt-2 text-sm leading-6 text-stone-700">Manifest, metadata SQLite, fingerprint, struktur direktori, dan audit awal sudah tervalidasi. Import dataset tetap nonaktif sampai task ingestion.</p>
              </div>
            </div>
          </div>
        </section>
        <SystemStatus workspaceStatus="Terverifikasi" />
      </DashboardGrid>
    </div>
  );
}

function ProjectMetric({ label, value }: { readonly label: string; readonly value: string }) {
  return (
    <article className="rounded-2xl border border-stone-200 bg-white p-5 shadow-sm">
      <p className="text-xs font-bold uppercase tracking-[0.14em] text-stone-500">{label}</p>
      <p className="mt-3 text-base font-bold text-stone-950">{value}</p>
    </article>
  );
}

function ProjectErrorDashboard({ error, onDismiss, onRetry }: {
  readonly error: DesktopError;
  readonly onDismiss: () => void;
  readonly onRetry: () => void;
}) {
  const recovery = error.code === "PROJECT_CORRUPTED";
  return (
    <section aria-labelledby="project-error-title" className="grid min-h-[65vh] place-items-center">
      <div className="w-full max-w-2xl rounded-2xl border border-red-200 bg-white p-8 shadow-sm">
        <span className="grid size-12 place-items-center rounded-xl bg-red-50 text-red-700" aria-hidden="true">{recovery ? <ShieldAlert size={24} /> : <CircleAlert size={24} />}</span>
        <p className="mt-5 text-xs font-bold uppercase tracking-[0.14em] text-red-700">{error.code}</p>
        <h1 className="mt-2 text-xl font-bold" id="project-error-title">{recovery ? "Proyek memerlukan pemulihan" : error.message}</h1>
        <p className="mt-2 text-sm leading-6 text-stone-600">{error.message}</p>
        {error.remediation === undefined ? null : <p className="mt-4 rounded-lg bg-stone-100 px-4 py-3 text-sm font-medium leading-6 text-stone-700">{error.remediation}</p>}
        {error.field_errors.length === 0 ? null : (
          <ul className="mt-4 list-disc space-y-1 pl-5 text-sm text-red-800">{error.field_errors.map((item) => <li key={item}>{item}</li>)}</ul>
        )}
        <p className="mt-5 text-xs text-stone-500">Correlation ID: <code>{error.correlation_id}</code></p>
        <div className="mt-6 flex flex-wrap gap-3">
          <button className="rounded-lg border border-stone-300 px-4 py-2.5 text-sm font-semibold text-stone-700 hover:bg-stone-50" onClick={onDismiss} type="button">Kembali</button>
          {error.retriable ? <button className="inline-flex items-center gap-2 rounded-lg bg-emerald-800 px-4 py-2.5 text-sm font-semibold text-white hover:bg-emerald-900" onClick={onRetry} type="button"><RefreshCw size={16} aria-hidden="true" /> Coba lagi</button> : null}
        </div>
      </div>
    </section>
  );
}

function PermissionDashboard({ error, onDismiss }: { readonly error: DesktopError; readonly onDismiss: () => void }) {
  const runtimeUnavailable = error.detail === "native desktop project runtime is unavailable";
  return (
    <section aria-labelledby="permission-title" className="grid min-h-[65vh] place-items-center">
      <div className="w-full max-w-xl rounded-2xl border border-amber-200 bg-white p-8 text-center shadow-sm">
        <span className="mx-auto grid size-12 place-items-center rounded-xl bg-amber-50 text-amber-800" aria-hidden="true"><LockKeyhole size={24} /></span>
        <h1 className="mt-5 text-xl font-bold" id="permission-title">Akses lokasi diperlukan</h1>
        <p className="mt-2 text-sm leading-6 text-stone-600">{error.message}</p>
        {error.remediation === undefined ? null : <p className="mt-4 text-sm font-medium leading-6 text-stone-700">{error.remediation}</p>}
        {runtimeUnavailable ? null : <button className="mt-6 rounded-lg bg-emerald-800 px-4 py-2.5 text-sm font-semibold text-white hover:bg-emerald-900" onClick={onDismiss} type="button">Pilih lokasi lain</button>}
      </div>
    </section>
  );
}

function DashboardGrid({ children }: { readonly children: React.ReactNode }) {
  return <div className="grid gap-6 xl:grid-cols-[minmax(0,1.65fr)_minmax(20rem,0.75fr)]">{children}</div>;
}

function SystemStatus({ workspaceStatus }: { readonly workspaceStatus: string }) {
  return (
    <aside aria-labelledby="system-status-title" className="rounded-2xl border border-stone-200 bg-white p-6 shadow-sm">
      <div className="flex items-center gap-2"><Activity className="text-emerald-800" size={18} aria-hidden="true" /><h2 className="font-bold" id="system-status-title">Status sistem</h2></div>
      <dl className="mt-6 space-y-5">
        <StatusRow label="Desktop shell" status="Siap" tone="ready" />
        <StatusRow label="Ruang kerja" status={workspaceStatus} tone={workspaceStatus === "Terverifikasi" ? "ready" : "neutral"} />
        <StatusRow label="Engine integration" status="Belum diaktifkan" tone="neutral" />
      </dl>
      <div className="mt-6 border-t border-stone-200 pt-5"><p className="text-xs font-bold uppercase tracking-[0.14em] text-stone-500">Kontrol integritas</p><p className="mt-3 text-sm leading-6 text-stone-600">Project lifecycle berjalan lokal melalui command native bertipe.</p></div>
    </aside>
  );
}

interface StatusRowProps {
  readonly label: string;
  readonly status: string;
  readonly tone: "neutral" | "ready";
}

function StatusRow({ label, status, tone }: StatusRowProps) {
  return (
    <div className="flex items-center justify-between gap-4">
      <dt className="text-sm text-stone-600">{label}</dt>
      <dd className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-bold ${tone === "ready" ? "bg-emerald-100 text-emerald-900" : "bg-stone-100 text-stone-600"}`}>
        <span className={`size-1.5 rounded-full ${tone === "ready" ? "bg-emerald-600" : "bg-stone-400"}`} aria-hidden="true" />{status}
      </dd>
    </div>
  );
}
