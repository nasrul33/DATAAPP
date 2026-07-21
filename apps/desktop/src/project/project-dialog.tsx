import { useEffect, useId, useRef, useState, type SyntheticEvent } from "react";
import { FolderPlus, LoaderCircle, X } from "lucide-react";

import type { ProjectActionStatus } from "./use-project-lifecycle";

interface ProjectDialogProps {
  readonly actionStatus: ProjectActionStatus;
  readonly onClose: () => void;
  readonly onCreate: (name: string) => Promise<boolean>;
}

export function ProjectDialog({ actionStatus, onClose, onCreate }: ProjectDialogProps) {
  const descriptionId = useId();
  const errorId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const [name, setName] = useState("");
  const [validationError, setValidationError] = useState<string | null>(null);
  const pending = actionStatus === "selecting" || actionStatus === "creating";

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  async function submit(event: SyntheticEvent<HTMLFormElement, SubmitEvent>) {
    event.preventDefault();
    const normalizedName = name.trim();
    if (normalizedName.length < 2 || normalizedName.length > 120) {
      setValidationError("Nama proyek harus terdiri dari 2 sampai 120 karakter.");
      return;
    }
    setValidationError(null);
    if (await onCreate(normalizedName)) onClose();
  }

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-emerald-950/55 p-4 backdrop-blur-sm"
      onKeyDown={(event) => {
        if (event.key === "Escape" && !pending) {
          onClose();
        }
      }}
      role="presentation"
    >
      <section
        aria-describedby={descriptionId}
        aria-labelledby="create-project-title"
        aria-modal="true"
        className="w-full max-w-lg rounded-2xl border border-stone-200 bg-white p-6 shadow-2xl shadow-emerald-950/25"
        role="dialog"
      >
        <div className="flex items-start justify-between gap-4">
          <span className="grid size-11 place-items-center rounded-xl bg-emerald-100 text-emerald-900" aria-hidden="true">
            <FolderPlus size={21} />
          </span>
          <button
            aria-label="Tutup dialog"
            className="grid size-9 place-items-center rounded-lg text-stone-500 hover:bg-stone-100 disabled:cursor-not-allowed disabled:opacity-50"
            disabled={pending}
            onClick={onClose}
            type="button"
          >
            <X size={18} aria-hidden="true" />
          </button>
        </div>
        <h2 className="mt-4 text-xl font-bold text-stone-950" id="create-project-title">Buat proyek analitik</h2>
        <p className="mt-2 text-sm leading-6 text-stone-600" id={descriptionId}>
          Beri nama proyek, lalu pilih lokasi lokal melalui dialog sistem. Teratai tidak akan mengubah data sumber.
        </p>

        <form className="mt-6" onSubmit={(event) => void submit(event)}>
          <label className="text-sm font-semibold text-stone-800" htmlFor="project-name">Nama proyek</label>
          <input
            aria-describedby={validationError === null ? undefined : errorId}
            aria-invalid={validationError !== null}
            className="mt-2 w-full rounded-lg border border-stone-300 bg-white px-3.5 py-2.5 text-sm text-stone-950 shadow-sm placeholder:text-stone-400 focus:border-emerald-700 focus:outline-none focus:ring-2 focus:ring-emerald-700/20"
            disabled={pending}
            id="project-name"
            maxLength={120}
            onChange={(event) => {
              setName(event.target.value);
            }}
            placeholder="Contoh: Audit Belanja 2026"
            ref={inputRef}
            value={name}
          />
          {validationError === null ? (
            <p className="mt-2 text-xs leading-5 text-stone-500">Lokasi akhir harus berupa direktori dengan ekstensi .teratai.</p>
          ) : (
            <p className="mt-2 text-xs font-semibold text-red-700" id={errorId}>{validationError}</p>
          )}

          <div className="mt-7 flex flex-col-reverse gap-3 sm:flex-row sm:justify-end">
            <button
              className="rounded-lg border border-stone-300 px-4 py-2.5 text-sm font-semibold text-stone-700 hover:bg-stone-50 disabled:cursor-not-allowed disabled:opacity-50"
              disabled={pending}
              onClick={onClose}
              type="button"
            >
              Batal
            </button>
            <button
              className="inline-flex items-center justify-center gap-2 rounded-lg bg-emerald-800 px-4 py-2.5 text-sm font-semibold text-white shadow-sm hover:bg-emerald-900 disabled:cursor-wait disabled:bg-emerald-700/65"
              disabled={pending || name.trim().length < 2}
              type="submit"
            >
              {pending ? <LoaderCircle className="animate-spin" size={16} aria-hidden="true" /> : <FolderPlus size={16} aria-hidden="true" />}
              {actionStatus === "creating" ? "Membuat proyek…" : actionStatus === "selecting" ? "Menunggu lokasi…" : "Pilih lokasi"}
            </button>
          </div>
        </form>
      </section>
    </div>
  );
}
