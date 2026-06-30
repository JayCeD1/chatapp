import React, { useState } from "react";
import { X, UserPlus, Check, Search } from "lucide-react";
import { DirectoryUser } from "../types";
import { initials, avatarColor } from "../utils";
import { useFocusTrap } from "../hooks/useFocusTrap";

interface InviteModalProps {
  roomName: string;
  users: DirectoryUser[];
  selfId: number;
  selfName: string;
  onAdd: (userId: number) => Promise<void> | void;
  onClose: () => void;
}

// Member picker for inviting people to a (private) channel. Adding is idempotent on the
// host, so we don't need the room's full member list — just hide yourself.
export const InviteModal: React.FC<InviteModalProps> = ({
  roomName,
  users,
  selfId,
  selfName,
  onAdd,
  onClose,
}) => {
  const trapRef = useFocusTrap<HTMLDivElement>(onClose);
  const [query, setQuery] = useState("");
  const [added, setAdded] = useState<Set<number>>(new Set());

  // Exclude self. In client mode currentUserId is the client's LOCAL id while the directory
  // carries the host's CANONICAL ids, so also match by name (the reliable cross-mode key).
  const candidates = users
    .filter((u) => u.id !== selfId && u.name !== selfName)
    .filter((u) => u.name.toLowerCase().includes(query.trim().toLowerCase()));

  const add = async (u: DirectoryUser) => {
    setAdded((s) => new Set(s).add(u.id));
    await onAdd(u.id);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-4 pt-[10vh]"
      onMouseDown={onClose}
      role="dialog"
      aria-modal="true"
      aria-label={`Add people to ${roomName}`}
    >
      <div
        ref={trapRef}
        className="w-full max-w-md bg-[var(--surface)] border border-[var(--border)] rounded-2xl shadow-2xl animate-scale-in overflow-hidden"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 h-14 border-b border-[var(--border)]">
          <h2 className="font-semibold text-[var(--text)] truncate">
            Add people to #{roomName}
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

        <div className="max-h-[50vh] overflow-y-auto scrollbar-thin scrollbar-track-transparent">
          {candidates.length === 0 ? (
            <p className="px-4 py-6 text-sm text-[var(--text-faint)] text-center">
              No one to add.
            </p>
          ) : (
            candidates.map((u) => (
              <div
                key={u.id}
                className="flex items-center gap-3 px-4 py-2 hover:bg-[var(--surface-2)] transition-colors"
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
                <button
                  onClick={() => add(u)}
                  disabled={added.has(u.id)}
                  className="flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium transition-colors disabled:opacity-60 disabled:cursor-default bg-[var(--surface-3)] text-[var(--text)] hover:bg-[var(--accent)] hover:text-white"
                >
                  {added.has(u.id) ? (
                    <>
                      <Check className="w-3.5 h-3.5" /> Added
                    </>
                  ) : (
                    <>
                      <UserPlus className="w-3.5 h-3.5" /> Add
                    </>
                  )}
                </button>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
};
