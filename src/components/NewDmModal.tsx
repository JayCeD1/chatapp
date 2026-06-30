import React, { useState } from "react";
import { X, Search, Check, MessageSquarePlus } from "lucide-react";
import { DirectoryUser } from "../types";
import { initials, avatarColor } from "../utils";
import { useFocusTrap } from "../hooks/useFocusTrap";

interface NewDmModalProps {
  users: DirectoryUser[];
  selfName: string;
  onStart: (targetIds: number[]) => Promise<void> | void;
  onClose: () => void;
}

// Picker for starting a direct message. Selecting one person opens a 1:1; selecting several
// opens a group conversation. Self is excluded by name (reliable across the local/canonical
// id split — the directory carries the host's canonical ids).
export const NewDmModal: React.FC<NewDmModalProps> = ({
  users,
  selfName,
  onStart,
  onClose,
}) => {
  const trapRef = useFocusTrap<HTMLDivElement>(onClose);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<number>>(new Set());

  const candidates = users
    .filter((u) => u.name !== selfName)
    .filter((u) => u.name.toLowerCase().includes(query.trim().toLowerCase()));

  const toggle = (id: number) =>
    setSelected((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const start = async () => {
    if (selected.size === 0) return;
    await onStart(Array.from(selected));
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-4 pt-[10vh]"
      onMouseDown={onClose}
      role="dialog"
      aria-modal="true"
      aria-label="Start a direct message"
    >
      <div
        ref={trapRef}
        className="w-full max-w-md bg-[var(--surface)] border border-[var(--border)] rounded-2xl shadow-2xl animate-scale-in overflow-hidden"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 h-14 border-b border-[var(--border)]">
          <h2 className="font-semibold text-[var(--text)] truncate">
            New message
          </h2>
          <button
            onClick={onClose}
            aria-label="Close"
            className="p-1.5 rounded-md text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="flex items-center gap-2 px-4 h-12 border-b border-[var(--border)]">
          <Search className="w-4 h-4 text-[var(--text-faint)] shrink-0" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search people…"
            className="flex-1 bg-transparent text-sm text-[var(--text)] placeholder-[var(--text-faint)] focus:outline-none"
          />
        </div>

        <div className="max-h-[45vh] overflow-y-auto scrollbar-thin scrollbar-track-transparent">
          {candidates.length === 0 ? (
            <p className="px-4 py-6 text-sm text-[var(--text-faint)] text-center">
              No one to message.
            </p>
          ) : (
            candidates.map((u) => {
              const on = selected.has(u.id);
              return (
                <button
                  key={u.id}
                  onClick={() => toggle(u.id)}
                  aria-pressed={on}
                  className="w-full flex items-center gap-3 px-4 py-2 hover:bg-[var(--surface-2)] transition-colors text-left"
                >
                  <div
                    className="relative flex items-center justify-center w-8 h-8 rounded-full text-[11px] font-semibold text-white shrink-0"
                    style={{ background: avatarColor(u.name) }}
                  >
                    {initials(u.name)}
                    {u.is_online && (
                      <span className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-[var(--online)] border-2 border-[var(--surface)]" />
                    )}
                  </div>
                  <span className="flex-1 truncate text-sm text-[var(--text)]">
                    {u.name}
                  </span>
                  <span
                    className={`flex items-center justify-center w-5 h-5 rounded-md border transition-colors ${
                      on
                        ? "bg-[var(--accent)] border-[var(--accent)] text-white"
                        : "border-[var(--border)]"
                    }`}
                  >
                    {on && <Check className="w-3.5 h-3.5" />}
                  </span>
                </button>
              );
            })
          )}
        </div>

        <div className="flex items-center justify-between gap-3 px-4 h-14 border-t border-[var(--border)]">
          <span className="text-xs text-[var(--text-faint)]">
            {selected.size === 0
              ? "Pick one or more people"
              : `${selected.size} selected`}
          </span>
          <button
            onClick={start}
            disabled={selected.size === 0}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium transition-colors disabled:opacity-50 disabled:cursor-default bg-[var(--accent)] text-white hover:bg-[var(--accent-strong)]"
          >
            <MessageSquarePlus className="w-4 h-4" />
            {selected.size > 1 ? "Start group" : "Message"}
          </button>
        </div>
      </div>
    </div>
  );
};
