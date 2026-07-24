import { useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type SyntheticEvent } from "react";
import { AlertTriangle, DatabaseBackup, LoaderCircle, ShieldCheck, X } from "lucide-react";

import type { DesktopError, ProjectDescriptor } from "@teratai/contracts";

import type { ProjectLifecycle } from "./use-project-lifecycle";

interface ProjectUpgradePanelProps {
  readonly actionStatus: ProjectLifecycle["actionStatus"];
  readonly error: DesktopError | null;
  readonly onDismissError: () => void;
  readonly onUpgrade: () => Promise<boolean>;
  readonly project: ProjectDescriptor;
}

interface ProjectUpgradeDialogProps {
  readonly confirmationValue: string;
  readonly error: DesktopError | null;
  readonly onCancel: () => void;
  readonly onConfirmationChange: (value: string) => void;
  readonly onDismissError: () => void;
  readonly onRetry: () => void;
  readonly onSubmit: () => void;
  readonly pending: boolean;
  readonly projectName: string;
}

export function projectNameMatchesExactly(
  confirmationValue: string,
  projectName: string,
): boolean {
  return confirmationValue === projectName;
}

export function getNextFocusIndex(
  currentIndex: number,
  focusableCount: number,
  movingBackward: boolean,
): number | null {
  if (focusableCount === 0) return null;
  if (movingBackward && currentIndex <= 0) return focusableCount - 1;
  if (!movingBackward && currentIndex >= focusableCount - 1) return 0;
  return null;
}

export function ProjectUpgradePanel({
  actionStatus,
  error,
  onDismissError,
  onUpgrade,
  project,
}: ProjectUpgradePanelProps) {
  const [confirmationValue, setConfirmationValue] = useState("");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [announcement, setAnnouncement] = useState("");
  const triggerRef = useRef<HTMLButtonElement>(null);
  const shouldRestoreFocusRef = useRef(false);
  const pending = actionStatus === "upgrading";

  useEffect(() => {
    if (dialogOpen || !shouldRestoreFocusRef.current) return;
    shouldRestoreFocusRef.current = false;
    triggerRef.current?.focus();
  }, [dialogOpen]);

  function openDialog() {
    if (actionStatus !== "idle") return;
    setAnnouncement("");
    setDialogOpen(true);
  }

  function closeDialog() {
    if (pending) return;
    shouldRestoreFocusRef.current = true;
    setDialogOpen(false);
  }

  async function submitUpgrade() {
    if (pending || !projectNameMatchesExactly(confirmationValue, project.name)) return;

    const upgraded = await onUpgrade();
    if (upgraded) {
      shouldRestoreFocusRef.current = true;
      setDialogOpen(false);
      setConfirmationValue("");
      setAnnouncement("Upgrade proyek selesai. Metadata schema sekarang versi 2.");
    }
  }

  return (
    <section aria-labelledby="project-upgrade-panel-title" className="rounded-2xl border border-amber-200 bg-amber-50 p-6 shadow-sm">
      <div className="flex flex-col gap-5 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex gap-4">
          <span className="grid size-11 shrink-0 place-items-center rounded-xl bg-amber-100 text-amber-900" aria-hidden="true">
            <DatabaseBackup size={21} />
          </span>
          <div>
            <p className="text-sm font-semibold text-amber-900">Upgrade proyek diperlukan</p>
            <h2 className="mt-1 text-xl font-bold text-stone-950" id="project-upgrade-panel-title">
              Metadata schema versi {project.metadata_schema_version} memerlukan versi 2
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-stone-700">
              Upgrade berlaku hanya untuk metadata proyek. Dataset sumber tetap tidak berubah. Tidak tersedia downgrade ke schema versi 1.
            </p>
          </div>
        </div>
        <button
          className="inline-flex shrink-0 items-center justify-center gap-2 rounded-lg bg-amber-800 px-4 py-2.5 text-sm font-semibold text-white shadow-sm hover:bg-amber-900 disabled:cursor-not-allowed disabled:bg-amber-800/60"
          disabled={actionStatus !== "idle"}
          onClick={openDialog}
          ref={triggerRef}
          type="button"
        >
          {pending ? <LoaderCircle className="animate-spin" size={16} aria-hidden="true" /> : <ShieldCheck size={16} aria-hidden="true" />}
          {pending ? "Meng-upgrade proyek…" : "Upgrade proyek"}
        </button>
      </div>

      {error === null || dialogOpen ? null : (
        <ProjectUpgradeError
          error={error}
          onDismiss={onDismissError}
          onRetry={openDialog}
          pending={pending}
        />
      )}

      <p aria-live="polite" className="sr-only" role="status">{announcement}</p>

      {dialogOpen ? (
        <ProjectUpgradeDialog
          confirmationValue={confirmationValue}
          error={error}
          onCancel={closeDialog}
          onConfirmationChange={setConfirmationValue}
          onDismissError={onDismissError}
          onRetry={() => void submitUpgrade()}
          onSubmit={() => void submitUpgrade()}
          pending={pending}
          projectName={project.name}
        />
      ) : null}
    </section>
  );
}

export function ProjectUpgradeDialog({
  confirmationValue,
  error,
  onCancel,
  onConfirmationChange,
  onDismissError,
  onRetry,
  onSubmit,
  pending,
  projectName,
}: ProjectUpgradeDialogProps) {
  const descriptionId = useId();
  const errorId = useId();
  const dialogRef = useRef<HTMLElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const confirmed = projectNameMatchesExactly(confirmationValue, projectName);

  useEffect(() => {
    if (pending) {
      dialogRef.current?.focus();
      return;
    }
    inputRef.current?.focus();
  }, [pending]);

  function submit(event: SyntheticEvent<HTMLFormElement, SubmitEvent>) {
    event.preventDefault();
    if (!pending && confirmed) onSubmit();
  }

  function containFocus(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key !== "Tab") return;
    const focusableElements = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), [data-focus-container="true"][tabindex="0"]',
      ),
    );
    const currentIndex = focusableElements.indexOf(event.currentTarget.ownerDocument.activeElement as HTMLElement);
    const nextFocusIndex = getNextFocusIndex(currentIndex, focusableElements.length, event.shiftKey);
    if (nextFocusIndex === null) return;

    event.preventDefault();
    focusableElements[nextFocusIndex]?.focus();
  }

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-emerald-950/55 p-4 backdrop-blur-sm"
      onKeyDown={(event) => {
        containFocus(event);
        if (event.key === "Escape" && !pending) onCancel();
      }}
      role="presentation"
    >
      <section
        aria-busy={pending}
        aria-describedby={error === null ? descriptionId : `${descriptionId} ${errorId}`}
        aria-labelledby="project-upgrade-dialog-title"
        aria-modal="true"
        className="w-full max-w-xl rounded-2xl border border-stone-200 bg-white p-6 shadow-2xl shadow-emerald-950/25"
        role="alertdialog"
        ref={dialogRef}
        tabIndex={pending ? 0 : -1}
        data-focus-container="true"
      >
        <div className="flex items-start justify-between gap-4">
          <span className="grid size-11 place-items-center rounded-xl bg-amber-100 text-amber-900" aria-hidden="true">
            <AlertTriangle size={21} />
          </span>
          <button
            aria-label="Tutup dialog upgrade"
            className="grid size-9 place-items-center rounded-lg text-stone-500 hover:bg-stone-100 disabled:cursor-not-allowed disabled:opacity-50"
            disabled={pending}
            onClick={onCancel}
            type="button"
          >
            <X size={18} aria-hidden="true" />
          </button>
        </div>
        <h2 className="mt-4 text-xl font-bold text-stone-950" id="project-upgrade-dialog-title">Konfirmasi upgrade metadata</h2>
        <div className="mt-2 space-y-2 text-sm leading-6 text-stone-700" id={descriptionId}>
          <p>Ketik nama proyek persis seperti tercantum untuk mengonfirmasi upgrade metadata dari schema versi 1 ke versi 2.</p>
          <p>Tidak ada downgrade. Bukti pemulihan dibuat sebelum migrasi, dataset sumber tidak berubah, dan aplikasi harus tetap terbuka selama migrasi terbatas berlangsung.</p>
        </div>

        {error === null ? null : (
          <ProjectUpgradeError
            error={error}
            id={errorId}
            onDismiss={onDismissError}
            onRetry={onRetry}
            pending={pending}
          />
        )}

        <form className="mt-6" onSubmit={submit}>
          <label className="text-sm font-semibold text-stone-800" htmlFor="project-upgrade-confirmation">
            Nama proyek: <span className="font-bold text-stone-950">{projectName}</span>
          </label>
          <input
            autoComplete="off"
            className="mt-2 w-full rounded-lg border border-stone-300 bg-white px-3.5 py-2.5 text-sm text-stone-950 shadow-sm placeholder:text-stone-400 focus:border-amber-700 focus:outline-none focus:ring-2 focus:ring-amber-700/20 disabled:cursor-not-allowed disabled:bg-stone-100"
            disabled={pending}
            id="project-upgrade-confirmation"
            onChange={(event) => {
              onConfirmationChange(event.target.value);
            }}
            ref={inputRef}
            spellCheck={false}
            value={confirmationValue}
          />
          <p className="mt-2 text-xs leading-5 text-stone-500">Spasi, kapitalisasi, dan karakter Unicode harus identik.</p>

          <div className="mt-7 flex flex-col-reverse gap-3 sm:flex-row sm:justify-end">
            <button
              className="rounded-lg border border-stone-300 px-4 py-2.5 text-sm font-semibold text-stone-700 hover:bg-stone-50 disabled:cursor-not-allowed disabled:opacity-50"
              disabled={pending}
              onClick={onCancel}
              type="button"
            >
              Batal
            </button>
            <button
              className="inline-flex items-center justify-center gap-2 rounded-lg bg-amber-800 px-4 py-2.5 text-sm font-semibold text-white shadow-sm hover:bg-amber-900 disabled:cursor-not-allowed disabled:bg-amber-800/60"
              disabled={pending || !confirmed}
              type="submit"
            >
              {pending ? <LoaderCircle className="animate-spin" size={16} aria-hidden="true" /> : <ShieldCheck size={16} aria-hidden="true" />}
              {pending ? "Meng-upgrade proyek…" : "Upgrade proyek"}
            </button>
          </div>
        </form>
      </section>
    </div>
  );
}

interface ProjectUpgradeErrorProps {
  readonly error: DesktopError;
  readonly id?: string;
  readonly onDismiss: () => void;
  readonly onRetry: () => void;
  readonly pending: boolean;
}

function ProjectUpgradeError({ error, id, onDismiss, onRetry, pending }: ProjectUpgradeErrorProps) {
  const recoveryRequired = error.code === "PROJECT_CORRUPTED";

  return (
    <div className="mt-5 rounded-xl border border-red-200 bg-red-50 p-4" id={id} role="alert">
      <p className="text-sm font-semibold text-red-900">
        {recoveryRequired ? "Proyek memerlukan pemulihan" : "Upgrade proyek belum selesai"}
      </p>
      <p className="mt-1 text-sm leading-6 text-red-800">{error.message}</p>
      {error.remediation === undefined ? null : (
        <p className="mt-2 text-sm leading-6 text-red-800">{error.remediation}</p>
      )}
      {error.field_errors.length === 0 ? null : (
        <ul className="mt-2 list-disc space-y-1 pl-5 text-sm leading-6 text-red-800">
          {error.field_errors.map((fieldError) => <li key={fieldError}>{fieldError}</li>)}
        </ul>
      )}
      <p className="mt-3 text-xs font-medium text-red-800">ID korelasi: {error.correlation_id}</p>
      {error.retriable && !recoveryRequired ? (
        <div className="mt-4 flex flex-wrap gap-3">
          <button
            className="rounded-lg border border-red-300 px-3.5 py-2 text-sm font-semibold text-red-900 hover:bg-red-100 disabled:cursor-not-allowed disabled:opacity-50"
            disabled={pending}
            onClick={onDismiss}
            type="button"
          >
            Tutup pesan
          </button>
          <button
            className="rounded-lg bg-red-800 px-3.5 py-2 text-sm font-semibold text-white hover:bg-red-900 disabled:cursor-not-allowed disabled:bg-red-800/60"
            disabled={pending}
            onClick={onRetry}
            type="button"
          >
            Coba lagi
          </button>
        </div>
      ) : null}
    </div>
  );
}
