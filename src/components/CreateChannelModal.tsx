import React, { useState } from "react";
import { X, Hash, AlertCircle } from "lucide-react";
import { Department } from "../types";

interface CreateChannelModalProps {
  departments: Department[];
  defaultDepartmentId: number | null;
  onCreate: (
    name: string,
    description: string,
    departmentId: number | null,
    isPrivate: boolean,
  ) => Promise<void>;
  onClose: () => void;
}

export const CreateChannelModal: React.FC<CreateChannelModalProps> = ({
  departments,
  defaultDepartmentId,
  onCreate,
  onClose,
}) => {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [departmentId, setDepartmentId] = useState<number | null>(
    defaultDepartmentId,
  );
  const [isPrivate, setIsPrivate] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await onCreate(name.trim(), description.trim(), departmentId, isPrivate);
      onClose();
    } catch (err) {
      setError(
        String(err).replace(/^.*?:\s*/, "") || "Couldn't create the channel.",
      );
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onMouseDown={onClose}
      role="dialog"
      aria-modal="true"
      aria-label="Create a channel"
    >
      <div
        className="w-full max-w-md bg-[var(--surface)] border border-[var(--border)] rounded-2xl shadow-2xl animate-scale-in"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 h-14 border-b border-[var(--border)]">
          <h2 className="font-semibold text-[var(--text)]">Create a channel</h2>
          <button
            onClick={onClose}
            aria-label="Close"
            className="p-1.5 rounded-md text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <form onSubmit={submit} className="p-5 space-y-4">
          <div>
            <label
              htmlFor="ch-name"
              className="block text-xs font-medium text-[var(--text-dim)] mb-1.5"
            >
              Channel name
            </label>
            <div className="relative">
              <Hash className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-[var(--text-faint)]" />
              <input
                id="ch-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. design-team"
                autoFocus
                maxLength={64}
                className="w-full bg-[var(--surface-2)] border border-[var(--border)] rounded-xl py-2.5 pl-9 pr-3 text-[var(--text)] placeholder-[var(--text-faint)] focus:outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent-soft)] transition-colors"
              />
            </div>
          </div>

          <div>
            <label
              htmlFor="ch-desc"
              className="block text-xs font-medium text-[var(--text-dim)] mb-1.5"
            >
              Description{" "}
              <span className="text-[var(--text-faint)]">(optional)</span>
            </label>
            <input
              id="ch-desc"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="What's this channel about?"
              maxLength={200}
              className="w-full bg-[var(--surface-2)] border border-[var(--border)] rounded-xl py-2.5 px-3 text-[var(--text)] placeholder-[var(--text-faint)] focus:outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent-soft)] transition-colors"
            />
          </div>

          <div>
            <label
              htmlFor="ch-dept"
              className="block text-xs font-medium text-[var(--text-dim)] mb-1.5"
            >
              Department
            </label>
            <select
              id="ch-dept"
              value={departmentId ?? ""}
              onChange={(e) =>
                setDepartmentId(e.target.value ? Number(e.target.value) : null)
              }
              className="w-full bg-[var(--surface-2)] border border-[var(--border)] rounded-xl py-2.5 px-3 text-[var(--text)] focus:outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent-soft)] transition-colors appearance-none cursor-pointer"
            >
              <option value="">No department</option>
              {departments.map((dep) => (
                <option key={dep.id} value={dep.id}>
                  {dep.name}
                </option>
              ))}
            </select>
          </div>

          <label className="flex items-center gap-2.5 cursor-pointer select-none">
            <input
              type="checkbox"
              checked={isPrivate}
              onChange={(e) => setIsPrivate(e.target.checked)}
              className="w-4 h-4 accent-[var(--accent)]"
            />
            <span className="text-sm text-[var(--text-dim)]">
              Private channel
            </span>
          </label>

          {error && (
            <div
              role="alert"
              className="flex items-start gap-2 text-sm text-[var(--danger)] bg-[var(--danger)]/10 border border-[var(--danger)]/30 rounded-lg px-3 py-2"
            >
              <AlertCircle className="w-4 h-4 mt-0.5 shrink-0" />
              <span>{error}</span>
            </div>
          )}

          <div className="flex justify-end gap-2 pt-1">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 rounded-lg text-sm text-[var(--text-dim)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={!name.trim() || submitting}
              className="px-4 py-2 rounded-lg text-sm font-semibold bg-[var(--accent)] text-white hover:bg-[var(--accent-strong)] disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            >
              {submitting ? "Creating…" : "Create channel"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
