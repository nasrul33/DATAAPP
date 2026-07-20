import {
  Activity,
  BarChart3,
  BriefcaseBusiness,
  ChevronDown,
  CircleAlert,
  Database,
  FileSearch,
  Flower2,
  FolderOpen,
  HardDrive,
  LayoutDashboard,
  LoaderCircle,
  PanelLeft,
  RefreshCw,
  Settings,
  Share2,
  ShieldCheck,
  Sparkles,
  Workflow,
} from "lucide-react";

import type { StartupState } from "./app";

interface AppShellProps {
  readonly onRetry?: (() => void) | undefined;
  readonly startupState: StartupState;
}

const navigation = [
  { icon: LayoutDashboard, label: "Dashboard", status: "active" },
  { icon: BriefcaseBusiness, label: "Proyek", status: "planned" },
  { icon: Database, label: "Dataset", status: "planned" },
  { icon: Workflow, label: "Workflow", status: "planned" },
  { icon: FileSearch, label: "Temuan", status: "planned" },
  { icon: Share2, label: "Ekspor", status: "planned" },
] as const;

export function AppShell({ onRetry, startupState }: AppShellProps) {
  return (
    <div className="min-h-screen bg-stone-100 text-stone-950">
      <a className="skip-link" href="#main-content">
        Lewati ke konten utama
      </a>

      <div className="grid min-h-screen lg:grid-cols-[17rem_1fr]">
        <aside className="flex flex-col border-r border-emerald-950/50 bg-[#08251d] px-4 py-5 text-emerald-50" aria-label="Navigasi utama">
          <div className="flex items-center gap-3 px-2">
            <span className="grid size-10 place-items-center rounded-xl bg-emerald-400 text-emerald-950 shadow-lg shadow-emerald-950/30" aria-hidden="true">
              <Flower2 size={22} strokeWidth={2.4} />
            </span>
            <div>
              <p className="text-sm font-semibold tracking-wide">Teratai</p>
              <p className="text-xs text-emerald-200/70">Analytics Desktop</p>
            </div>
          </div>

          <nav className="mt-9" aria-label="Area aplikasi">
            <p className="px-3 text-[0.68rem] font-bold uppercase tracking-[0.18em] text-emerald-200/45">Ruang kerja</p>
            <ul className="mt-3 space-y-1">
              {navigation.map(({ icon: Icon, label, status }) => (
                <li key={label}>
                  <button
                    type="button"
                    aria-current={status === "active" ? "page" : undefined}
                    aria-disabled={status === "planned"}
                    disabled={status === "planned"}
                    className={`flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left text-sm font-medium transition ${
                      status === "active"
                        ? "bg-emerald-400 text-emerald-950 shadow-sm"
                        : "text-emerald-100/65 disabled:cursor-not-allowed disabled:hover:bg-transparent"
                    }`}
                  >
                    <Icon size={17} aria-hidden="true" />
                    <span>{label}</span>
                    {status === "planned" ? (
                      <span className="ml-auto rounded-full border border-emerald-200/15 px-2 py-0.5 text-[0.62rem] font-bold uppercase tracking-wide text-emerald-100/45">
                        Segera
                      </span>
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
            <button type="button" disabled className="flex w-full cursor-not-allowed items-center gap-3 rounded-lg px-3 py-2.5 text-sm font-medium text-emerald-100/45">
              <Settings size={17} aria-hidden="true" />
              Pengaturan
            </button>
          </div>
        </aside>

        <div className="min-w-0">
          <header className="flex min-h-16 items-center justify-between border-b border-stone-200 bg-white/90 px-5 backdrop-blur md:px-8">
            <div className="flex items-center gap-3">
              <button type="button" disabled aria-label="Buka navigasi" className="grid size-9 cursor-not-allowed place-items-center rounded-lg border border-stone-200 text-stone-400 lg:hidden">
                <PanelLeft size={18} aria-hidden="true" />
              </button>
              <div>
                <p className="text-sm font-semibold text-stone-900">Ruang kerja lokal</p>
                <p className="text-xs text-stone-500">Belum ada proyek aktif</p>
              </div>
            </div>
            <button type="button" disabled className="flex cursor-not-allowed items-center gap-2 rounded-lg border border-stone-200 bg-stone-50 px-3 py-2 text-xs font-semibold text-stone-500">
              <span className="size-2 rounded-full bg-amber-500" aria-hidden="true" />
              Mode lokal
              <ChevronDown size={14} aria-hidden="true" />
            </button>
          </header>

          <main id="main-content" className="mx-auto max-w-[96rem] p-5 md:p-8">
            {startupState === "loading" ? <LoadingDashboard /> : null}
            {startupState === "error" ? <ErrorDashboard onRetry={onRetry} /> : null}
            {startupState === "ready" ? <EmptyDashboard /> : null}
          </main>
        </div>
      </div>
    </div>
  );
}

function LoadingDashboard() {
  return (
    <section aria-busy="true" aria-label="Memuat ruang kerja" className="animate-pulse space-y-6">
      <div className="flex items-center gap-3 text-sm font-semibold text-emerald-800">
        <LoaderCircle className="animate-spin" size={18} aria-hidden="true" />
        Menyiapkan ruang kerja…
      </div>
      <div className="h-20 max-w-2xl rounded-2xl bg-stone-200" />
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {Array.from({ length: 4 }, (_, index) => (
          <div key={index} className="h-32 rounded-2xl border border-stone-200 bg-white" />
        ))}
      </div>
      <div className="h-80 rounded-2xl border border-stone-200 bg-white" />
    </section>
  );
}

function ErrorDashboard({ onRetry }: Pick<AppShellProps, "onRetry">) {
  return (
    <section className="grid min-h-[65vh] place-items-center" aria-labelledby="startup-error-title">
      <div className="w-full max-w-xl rounded-2xl border border-red-200 bg-white p-8 text-center shadow-sm">
        <span className="mx-auto grid size-12 place-items-center rounded-xl bg-red-50 text-red-700" aria-hidden="true">
          <CircleAlert size={24} />
        </span>
        <h1 id="startup-error-title" className="mt-5 text-xl font-bold text-stone-950">Ruang kerja gagal disiapkan</h1>
        <p className="mx-auto mt-2 max-w-md text-sm leading-6 text-stone-600">Teratai tidak dapat memuat status aplikasi lokal. Tidak ada data sumber yang diubah.</p>
        <button type="button" onClick={onRetry} className="mt-6 inline-flex items-center gap-2 rounded-lg bg-emerald-800 px-4 py-2.5 text-sm font-semibold text-white shadow-sm hover:bg-emerald-900 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-emerald-700">
          <RefreshCw size={16} aria-hidden="true" />
          Coba lagi
        </button>
      </div>
    </section>
  );
}

function EmptyDashboard() {
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
            <Sparkles size={14} aria-hidden="true" />
            Analitik yang dapat dijelaskan
          </div>
          <h1 id="dashboard-title" className="mt-2 text-3xl font-bold tracking-tight text-stone-950">Dashboard</h1>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-stone-600">Kelola analisis lokal, telusuri setiap transformasi, dan pertahankan data sumber tetap utuh.</p>
        </div>
        <button type="button" disabled aria-describedby="project-action-help" className="inline-flex cursor-not-allowed items-center justify-center gap-2 rounded-lg bg-stone-300 px-4 py-2.5 text-sm font-semibold text-stone-600">
          <FolderOpen size={17} aria-hidden="true" />
          Buat proyek
        </button>
        <span id="project-action-help" className="sr-only">Pembuatan proyek tersedia pada tahap T-0100.</span>
      </section>

      <section className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4" aria-label="Ringkasan ruang kerja">
        {metrics.map(({ icon: Icon, label, value }) => (
          <article key={label} className="rounded-2xl border border-stone-200 bg-white p-5 shadow-sm shadow-stone-950/[0.025]">
            <div className="flex items-center justify-between">
              <span className="grid size-9 place-items-center rounded-lg bg-emerald-50 text-emerald-800" aria-hidden="true"><Icon size={18} /></span>
              <span className="rounded-full bg-stone-100 px-2 py-1 text-[0.65rem] font-bold uppercase tracking-wide text-stone-500">Kosong</span>
            </div>
            <p className="mt-5 text-2xl font-bold tabular-nums text-stone-950">{value}</p>
            <p className="mt-1 text-sm text-stone-600">{label}</p>
          </article>
        ))}
      </section>

      <div className="grid gap-6 xl:grid-cols-[minmax(0,1.65fr)_minmax(20rem,0.75fr)]">
        <section className="rounded-2xl border border-stone-200 bg-white p-6 shadow-sm shadow-stone-950/[0.025]" aria-labelledby="empty-project-title">
          <div className="grid min-h-64 place-items-center rounded-xl border border-dashed border-stone-300 bg-stone-50/70 px-6 py-10 text-center">
            <div className="max-w-lg">
              <span className="mx-auto grid size-14 place-items-center rounded-2xl bg-emerald-100 text-emerald-900" aria-hidden="true"><BarChart3 size={27} /></span>
              <h2 id="empty-project-title" className="mt-5 text-lg font-bold text-stone-950">Belum ada proyek analitik</h2>
              <p className="mt-2 text-sm leading-6 text-stone-600">Proyek akan menjadi wadah untuk dataset, workflow, hasil analisis, dan jejak audit. Fitur pembuatan proyek hadir pada tahap berikutnya.</p>
              <div className="mt-5 inline-flex items-center gap-2 rounded-full border border-emerald-200 bg-emerald-50 px-3 py-1.5 text-xs font-semibold text-emerald-900">
                <ShieldCheck size={14} aria-hidden="true" />
                Data sumber tidak pernah diubah langsung
              </div>
            </div>
          </div>
        </section>

        <aside className="rounded-2xl border border-stone-200 bg-white p-6 shadow-sm shadow-stone-950/[0.025]" aria-labelledby="system-status-title">
          <div className="flex items-center gap-2">
            <Activity size={18} className="text-emerald-800" aria-hidden="true" />
            <h2 id="system-status-title" className="font-bold text-stone-950">Status sistem</h2>
          </div>
          <dl className="mt-6 space-y-5">
            <StatusRow label="Desktop shell" status="Siap" tone="ready" />
            <StatusRow label="Ruang kerja" status="Belum dipilih" tone="neutral" />
            <StatusRow label="Analytics engine" status="Menunggu T-0006" tone="neutral" />
          </dl>
          <div className="mt-6 border-t border-stone-200 pt-5">
            <p className="text-xs font-bold uppercase tracking-[0.14em] text-stone-500">Aktivitas terbaru</p>
            <p className="mt-3 text-sm leading-6 text-stone-600">Belum ada aktivitas. Perubahan terverifikasi akan muncul di sini.</p>
          </div>
        </aside>
      </div>
    </div>
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
        <span className={`size-1.5 rounded-full ${tone === "ready" ? "bg-emerald-600" : "bg-stone-400"}`} aria-hidden="true" />
        {status}
      </dd>
    </div>
  );
}
