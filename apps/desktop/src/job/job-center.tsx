import { useEffect, useMemo, useRef, useState } from "react";
import {
  Ban,
  CheckCircle2,
  CircleAlert,
  CircleDashed,
  Clock3,
  LoaderCircle,
  RefreshCw,
  RotateCcw,
  ShieldAlert,
  X,
  XCircle,
} from "lucide-react";

import type { JobDescriptor, ProjectDescriptor } from "@teratai/contracts";

import type { JobClient } from "./job-client";
import {
  canCancelJob,
  filterJobs,
  jobProgressPercent,
  JOB_STATUS_OPTIONS,
  MAX_VISIBLE_JOBS,
  type JobCenterState,
  type JobStatusFilter,
} from "./job-center-model";
import { useJobCenter, type JobCenterController } from "./use-job-center";

interface JobCenterProps {
  readonly client: JobClient | null;
  readonly project: ProjectDescriptor;
}

export function JobCenter({ client, project }: JobCenterProps) {
  const controller = useJobCenter(client, project);
  return <JobCenterView controller={controller} />;
}

export function JobCenterView({ controller }: { readonly controller: JobCenterController }) {
  const [statusFilter, setStatusFilter] = useState<JobStatusFilter>("ALL");
  const [confirmation, setConfirmation] = useState<JobDescriptor | null>(null);
  const cancelButtonRef = useRef<HTMLButtonElement>(null);
  const filteredJobs = useMemo(
    () => filterJobs(controller.items, statusFilter),
    [controller.items, statusFilter],
  );

  useEffect(() => {
    if (confirmation !== null) cancelButtonRef.current?.focus();
  }, [confirmation]);

  useEffect(() => {
    if (confirmation === null) return;
    const current = controller.items.find((job) => job.job_id === confirmation.job_id);
    if (current === undefined || !canCancelJob(current)) {
      setConfirmation(null);
    } else if (current.revision !== confirmation.revision) {
      setConfirmation(current);
    }
  }, [confirmation, controller.items]);

  if (controller.status === "loading") return <JobCenterLoading />;
  if (controller.status === "upgrade") return <JobCenterUpgrade />;
  if (controller.status === "unavailable") return <JobCenterUnavailable />;
  if (controller.status === "error" && controller.error !== null) {
    return <JobCenterInitialError controller={controller} />;
  }

  const summary = summarizeJobs(controller.items);
  return (
    <section aria-labelledby="job-center-title" className="rounded-2xl border border-stone-200 bg-white shadow-sm">
      <div className="flex flex-col gap-4 border-b border-stone-200 p-5 md:flex-row md:items-center md:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <h2 className="text-lg font-bold" id="job-center-title">Job Center</h2>
            {controller.refreshing ? (
              <span className="inline-flex items-center gap-1.5 text-xs font-semibold text-emerald-800" role="status">
                <LoaderCircle className="animate-spin" size={13} aria-hidden="true" /> Menyinkronkan
              </span>
            ) : null}
          </div>
          <p className="mt-1 text-sm text-stone-600">
            Status pekerjaan tersimpan untuk proyek ini. Maksimum {MAX_VISIBLE_JOBS} item ditampilkan.
          </p>
        </div>
        <div className="flex flex-col gap-2 sm:flex-row">
          <label className="flex items-center gap-2 text-sm font-semibold text-stone-700">
            <span>Status</span>
            <select
              className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm"
              onChange={(event) => {
                setStatusFilter(event.target.value as JobStatusFilter);
              }}
              value={statusFilter}
            >
              {JOB_STATUS_OPTIONS.map((status) => (
                <option key={status} value={status}>{statusLabel(status)}</option>
              ))}
            </select>
          </label>
          <button
            className="inline-flex items-center justify-center gap-2 rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm font-semibold text-stone-700 hover:bg-stone-50 disabled:cursor-wait disabled:opacity-60"
            disabled={controller.refreshing}
            onClick={() => void controller.refresh()}
            type="button"
          >
            {controller.refreshing ? <LoaderCircle className="animate-spin" size={16} aria-hidden="true" /> : <RefreshCw size={16} aria-hidden="true" />}
            Muat ulang
          </button>
        </div>
      </div>

      <div aria-label="Ringkasan pekerjaan termuat" className="grid grid-cols-2 gap-px border-b border-stone-200 bg-stone-200 lg:grid-cols-4">
        <SummaryMetric label="Termuat" value={controller.items.length} />
        <SummaryMetric label="Aktif" value={summary.active} />
        <SummaryMetric label="Selesai" value={summary.succeeded} />
        <SummaryMetric label="Gagal" value={summary.failed} tone={summary.failed > 0 ? "danger" : "default"} />
      </div>

      {controller.error === null ? null : (
        <InlineAlert
          message={controller.error.message}
          onDismiss={controller.clearError}
          {...(controller.error.remediation === undefined
            ? {}
            : { remediation: controller.error.remediation })}
          tone="danger"
        />
      )}
      {controller.eventWarning === null ? null : (
        <InlineAlert
          message="Pembaruan langsung terputus. Data tersimpan tetap dapat dimuat ulang."
          onDismiss={controller.clearEventWarning}
          remediation={controller.eventWarning}
          tone="warning"
        />
      )}

      {controller.items.length === 0 ? (
        <JobCenterEmpty />
      ) : filteredJobs.length === 0 ? (
        <div className="px-6 py-14 text-center">
          <CircleDashed className="mx-auto text-stone-400" size={32} aria-hidden="true" />
          <p className="mt-3 font-bold">Tidak ada status yang cocok</p>
          <p className="mt-1 text-sm text-stone-600">Filter hanya berlaku pada pekerjaan yang sudah termuat.</p>
        </div>
      ) : (
        <>
          <div className="hidden overflow-x-auto md:block">
            <JobTable
              cancelPendingIds={controller.cancelPendingIds}
              jobs={filteredJobs}
              onRequestCancel={setConfirmation}
            />
          </div>
          <div className="divide-y divide-stone-200 md:hidden">
            {filteredJobs.map((job) => (
                <JobCard
                cancelPending={controller.cancelPendingIds.has(job.job_id)}
                job={job}
                key={job.job_id}
                onRequestCancel={setConfirmation}
              />
            ))}
          </div>
        </>
      )}

      {controller.cursor === undefined ? null : (
        <div className="border-t border-stone-200 p-4 text-center">
          <button
            className="inline-flex items-center gap-2 rounded-lg border border-stone-300 px-4 py-2.5 text-sm font-semibold text-stone-700 hover:bg-stone-50 disabled:cursor-wait disabled:opacity-60"
            disabled={controller.loadingMore || controller.items.length >= MAX_VISIBLE_JOBS}
            onClick={() => void controller.loadMore()}
            type="button"
          >
            {controller.loadingMore ? <LoaderCircle className="animate-spin" size={16} aria-hidden="true" /> : <RotateCcw size={16} aria-hidden="true" />}
            {controller.items.length >= MAX_VISIBLE_JOBS ? "Batas tampilan tercapai" : "Muat lebih banyak"}
          </button>
        </div>
      )}

      {confirmation === null ? null : (
        <div
          aria-labelledby="cancel-job-title"
          aria-modal="true"
          className="fixed inset-0 z-50 grid place-items-center bg-stone-950/55 p-4"
          role="alertdialog"
        >
          <div className="w-full max-w-md rounded-2xl bg-white p-6 shadow-2xl">
            <span className="grid size-11 place-items-center rounded-xl bg-amber-100 text-amber-900" aria-hidden="true"><ShieldAlert size={22} /></span>
            <h3 className="mt-4 text-lg font-bold" id="cancel-job-title">Batalkan pekerjaan?</h3>
            <p className="mt-2 text-sm leading-6 text-stone-600">
              Permintaan bersifat kooperatif. Status akan menjadi “Membatalkan” sampai worker mengonfirmasi bahwa proses sudah berhenti.
            </p>
            <p className="mt-3 rounded-lg bg-stone-100 px-3 py-2 font-mono text-xs text-stone-700">{confirmation.kind}</p>
            <div className="mt-6 flex justify-end gap-3">
              <button
                className="rounded-lg border border-stone-300 px-4 py-2 text-sm font-semibold"
                onClick={() => {
                  setConfirmation(null);
                }}
                type="button"
              >
                Kembali
              </button>
              <button
                className="inline-flex items-center gap-2 rounded-lg bg-red-700 px-4 py-2 text-sm font-semibold text-white hover:bg-red-800"
                onClick={() => {
                  const selected = controller.items.find((job) => job.job_id === confirmation.job_id);
                  setConfirmation(null);
                  if (selected !== undefined && canCancelJob(selected)) {
                    void controller.cancel(selected);
                  }
                }}
                ref={cancelButtonRef}
                type="button"
              >
                <Ban size={16} aria-hidden="true" /> Ya, batalkan
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}

function JobCenterLoading() {
  return (
    <section aria-busy="true" aria-label="Memuat Job Center" className="rounded-2xl border border-stone-200 bg-white p-6 shadow-sm">
      <div className="flex items-center gap-2 text-sm font-semibold text-emerald-800">
        <LoaderCircle className="animate-spin" size={17} aria-hidden="true" /> Memuat pekerjaan tersimpan…
      </div>
      <div className="mt-5 space-y-3 animate-pulse">
        {Array.from({ length: 3 }, (_, index) => <div className="h-16 rounded-xl bg-stone-100" key={index} />)}
      </div>
    </section>
  );
}

function JobCenterUpgrade() {
  return (
    <section aria-labelledby="job-upgrade-title" className="rounded-2xl border border-amber-200 bg-amber-50/60 p-6 shadow-sm">
      <ShieldAlert className="text-amber-800" size={24} aria-hidden="true" />
      <h2 className="mt-3 font-bold" id="job-upgrade-title">Upgrade proyek diperlukan</h2>
      <p className="mt-2 max-w-3xl text-sm leading-6 text-stone-700">
        Job Center akan tersedia setelah metadata proyek diperbarui ke schema versi 2.
      </p>
    </section>
  );
}

function JobCenterUnavailable() {
  return (
    <section aria-labelledby="job-unavailable-title" className="rounded-2xl border border-stone-200 bg-white p-6 shadow-sm">
      <CircleAlert className="text-stone-500" size={24} aria-hidden="true" />
      <h2 className="mt-3 font-bold" id="job-unavailable-title">Job Center hanya tersedia di aplikasi desktop</h2>
      <p className="mt-2 text-sm leading-6 text-stone-600">Runtime browser tidak memiliki akses ke command proyek lokal.</p>
    </section>
  );
}

function JobCenterInitialError({ controller }: { readonly controller: JobCenterController }) {
  return (
    <section aria-labelledby="job-error-title" className="rounded-2xl border border-red-200 bg-white p-6 shadow-sm">
      <XCircle className="text-red-700" size={24} aria-hidden="true" />
      <h2 className="mt-3 font-bold" id="job-error-title">Pekerjaan tidak dapat dimuat</h2>
      <p className="mt-2 text-sm leading-6 text-stone-600">{controller.error?.message}</p>
      {controller.error?.remediation === undefined ? null : <p className="mt-2 text-sm font-medium text-stone-700">{controller.error.remediation}</p>}
      <button className="mt-5 inline-flex items-center gap-2 rounded-lg bg-emerald-800 px-4 py-2.5 text-sm font-semibold text-white hover:bg-emerald-900" onClick={() => void controller.refresh()} type="button">
        <RefreshCw size={16} aria-hidden="true" /> Coba lagi
      </button>
    </section>
  );
}

function JobCenterEmpty() {
  return (
    <div className="px-6 py-14 text-center">
      <Clock3 className="mx-auto text-stone-400" size={34} aria-hidden="true" />
      <p className="mt-3 font-bold">Belum ada pekerjaan</p>
      <p className="mt-1 text-sm text-stone-600">Pekerjaan latar belakang proyek ini akan tampil di sini.</p>
    </div>
  );
}

function JobTable({ cancelPendingIds, jobs, onRequestCancel }: {
  readonly cancelPendingIds: ReadonlySet<string>;
  readonly jobs: readonly JobDescriptor[];
  readonly onRequestCancel: (job: JobDescriptor) => void;
}) {
  return (
    <table className="w-full min-w-[56rem] border-collapse text-left">
      <thead className="bg-stone-50 text-xs font-bold uppercase tracking-wide text-stone-500">
        <tr>
          <th className="px-5 py-3" scope="col">Pekerjaan</th>
          <th className="px-5 py-3" scope="col">Status</th>
          <th className="px-5 py-3" scope="col">Progres</th>
          <th className="px-5 py-3" scope="col">Diperbarui</th>
          <th className="px-5 py-3 text-right" scope="col">Aksi</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-stone-200">
        {jobs.map((job) => (
          <JobTableRow
            cancelPending={cancelPendingIds.has(job.job_id)}
            job={job}
            key={job.job_id}
            onRequestCancel={onRequestCancel}
          />
        ))}
      </tbody>
    </table>
  );
}

function JobTableRow({ cancelPending, job, onRequestCancel }: JobRowProps) {
  return (
    <tr className="hover:bg-stone-50/70">
      <td className="px-5 py-4"><JobIdentity job={job} /></td>
      <td className="px-5 py-4"><JobStatusBadge status={job.status} /></td>
      <td className="w-64 px-5 py-4"><JobProgress job={job} /></td>
      <td className="whitespace-nowrap px-5 py-4 text-sm text-stone-600">{formatDateTime(job.updated_at)}</td>
      <td className="px-5 py-4 text-right"><CancelButton cancelPending={cancelPending} job={job} onRequestCancel={onRequestCancel} /></td>
    </tr>
  );
}

interface JobRowProps {
  readonly cancelPending: boolean;
  readonly job: JobDescriptor;
  readonly onRequestCancel: (job: JobDescriptor) => void;
}

function JobCard({ cancelPending, job, onRequestCancel }: JobRowProps) {
  return (
    <article className="space-y-4 p-5">
      <div className="flex items-start justify-between gap-3">
        <JobIdentity job={job} />
        <JobStatusBadge status={job.status} />
      </div>
      <JobProgress job={job} />
      <div className="flex items-center justify-between gap-3 border-t border-stone-100 pt-3">
        <span className="text-xs text-stone-500">{formatDateTime(job.updated_at)}</span>
        <CancelButton cancelPending={cancelPending} job={job} onRequestCancel={onRequestCancel} />
      </div>
    </article>
  );
}

function JobIdentity({ job }: { readonly job: JobDescriptor }) {
  return (
    <div className="min-w-0">
      <p className="truncate text-sm font-bold text-stone-900">{jobKindLabel(job.kind)}</p>
      <p className="mt-1 font-mono text-[0.68rem] text-stone-500" title={job.job_id}>{job.job_id.slice(0, 18)}… · rev {job.revision}</p>
      {job.error_message === undefined ? null : <p className="mt-2 max-w-md text-xs leading-5 text-red-700">{job.error_message}</p>}
    </div>
  );
}

function JobProgress({ job }: { readonly job: JobDescriptor }) {
  const percent = jobProgressPercent(job);
  if (percent === null) {
    return <p className="text-sm text-stone-600">{job.progress_message ?? (job.status === "QUEUED" ? "Menunggu worker" : "Progres belum tersedia")}</p>;
  }
  return (
    <div>
      <div className="mb-1.5 flex justify-between gap-3 text-xs text-stone-600">
        <span className="truncate">{job.progress_message ?? job.progress_phase ?? "Memproses"}</span>
        <span className="font-semibold tabular-nums">{percent}%</span>
      </div>
      <progress aria-label={`Progres ${jobKindLabel(job.kind)}`} className="h-2 w-full accent-emerald-700" max={100} value={percent}>{percent}%</progress>
    </div>
  );
}

function JobStatusBadge({ status }: { readonly status: string }) {
  const style = statusStyle(status);
  return (
    <span className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-1 text-xs font-bold ${style.className}`}>
      <style.Icon className={style.spin ? "animate-spin" : undefined} size={13} aria-hidden="true" />
      {statusLabel(status)}
    </span>
  );
}

function CancelButton({ cancelPending, job, onRequestCancel }: JobRowProps) {
  if (!canCancelJob(job)) {
    return <span className="text-xs font-medium text-stone-400">{job.status === "CANCELLING" ? "Menunggu worker" : "Tidak ada aksi"}</span>;
  }
  return (
    <button
      className="inline-flex items-center gap-1.5 rounded-lg border border-red-200 px-3 py-2 text-xs font-bold text-red-700 hover:bg-red-50 disabled:cursor-wait disabled:opacity-60"
      disabled={cancelPending}
      onClick={() => {
        onRequestCancel(job);
      }}
      type="button"
    >
      {cancelPending ? <LoaderCircle className="animate-spin" size={14} aria-hidden="true" /> : <Ban size={14} aria-hidden="true" />}
      {cancelPending ? "Meminta batal" : "Batalkan"}
    </button>
  );
}

function SummaryMetric({ label, tone = "default", value }: {
  readonly label: string;
  readonly tone?: "danger" | "default";
  readonly value: number;
}) {
  return (
    <div className="bg-white px-5 py-4">
      <p className="text-xs font-bold uppercase tracking-wide text-stone-500">{label}</p>
      <p className={`mt-1 text-xl font-bold tabular-nums ${tone === "danger" ? "text-red-700" : "text-stone-950"}`}>{value}</p>
    </div>
  );
}

function InlineAlert({ message, onDismiss, remediation, tone }: {
  readonly message: string;
  readonly onDismiss: () => void;
  readonly remediation?: string;
  readonly tone: "danger" | "warning";
}) {
  const danger = tone === "danger";
  return (
    <div aria-live="polite" className={`flex items-start gap-3 border-b px-5 py-3 ${danger ? "border-red-200 bg-red-50 text-red-900" : "border-amber-200 bg-amber-50 text-amber-950"}`} role="status">
      <CircleAlert className="mt-0.5 shrink-0" size={17} aria-hidden="true" />
      <div className="min-w-0 flex-1 text-sm">
        <p className="font-semibold">{message}</p>
        {remediation === undefined ? null : <p className="mt-1 text-xs leading-5 opacity-80">{remediation}</p>}
      </div>
      <button aria-label="Tutup pemberitahuan" className="rounded p-1 hover:bg-black/5" onClick={onDismiss} type="button"><X size={15} aria-hidden="true" /></button>
    </div>
  );
}

function summarizeJobs(items: readonly JobDescriptor[]) {
  return {
    active: items.filter((job) => job.status === "QUEUED" || job.status === "RUNNING" || job.status === "CANCELLING").length,
    failed: items.filter((job) => job.status === "FAILED").length,
    succeeded: items.filter((job) => job.status === "SUCCEEDED").length,
  };
}

function statusLabel(status: string): string {
  const labels: Readonly<Record<string, string>> = {
    ALL: "Semua",
    CANCELLED: "Dibatalkan",
    CANCELLING: "Membatalkan",
    FAILED: "Gagal",
    QUEUED: "Antre",
    RUNNING: "Berjalan",
    SUCCEEDED: "Selesai",
  };
  return labels[status] ?? status;
}

function statusStyle(status: string) {
  if (status === "RUNNING") return { className: "bg-blue-100 text-blue-900", Icon: LoaderCircle, spin: true };
  if (status === "SUCCEEDED") return { className: "bg-emerald-100 text-emerald-900", Icon: CheckCircle2, spin: false };
  if (status === "FAILED") return { className: "bg-red-100 text-red-900", Icon: XCircle, spin: false };
  if (status === "CANCELLING") return { className: "bg-amber-100 text-amber-900", Icon: LoaderCircle, spin: true };
  if (status === "CANCELLED") return { className: "bg-stone-200 text-stone-700", Icon: Ban, spin: false };
  return { className: "bg-violet-100 text-violet-900", Icon: Clock3, spin: false };
}

function jobKindLabel(kind: string): string {
  return kind
    .split(/[._-]/u)
    .filter((part) => part.length > 0)
    .map((part) => `${part.charAt(0).toLocaleUpperCase("id-ID")}${part.slice(1)}`)
    .join(" ");
}

function formatDateTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Waktu tidak valid";
  return new Intl.DateTimeFormat("id-ID", {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: "Asia/Jakarta",
  }).format(date);
}

export type { JobCenterState };
