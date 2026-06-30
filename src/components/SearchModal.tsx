import React, { useState, useEffect, useRef } from "react";
import { Search, X, Hash, Loader2 } from "lucide-react";
import { SearchResult } from "../types";
import { initials, avatarColor, formatSearchTime } from "../utils";

interface SearchModalProps {
  onSearch: (query: string) => Promise<SearchResult[]>;
  onJump: (roomId: number) => void;
  onClose: () => void;
}

export const SearchModal: React.FC<SearchModalProps> = ({
  onSearch,
  onJump,
  onClose,
}) => {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // Debounced search; a request token guards against out-of-order responses.
  const reqId = useRef(0);
  useEffect(() => {
    const q = query.trim();
    if (timer.current) clearTimeout(timer.current);
    if (q.length < 2) {
      setResults([]);
      setSearched(false);
      setLoading(false);
      return;
    }
    setLoading(true);
    const id = ++reqId.current;
    timer.current = setTimeout(async () => {
      const r = await onSearch(q);
      if (id !== reqId.current) return; // a newer query superseded this one
      setResults(r);
      setSearched(true);
      setLoading(false);
    }, 250);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [query, onSearch]);

  const pick = (r: SearchResult) => {
    onJump(r.room_id);
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-4 pt-[10vh]"
      onMouseDown={onClose}
      role="dialog"
      aria-modal="true"
      aria-label="Search messages"
    >
      <div
        className="w-full max-w-lg bg-[var(--surface)] border border-[var(--border)] rounded-2xl shadow-2xl animate-scale-in overflow-hidden"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-4 h-14 border-b border-[var(--border)]">
          <Search className="w-4 h-4 text-[var(--text-faint)] shrink-0" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && onClose()}
            placeholder="Search messages…"
            className="flex-1 bg-transparent text-[var(--text)] placeholder-[var(--text-faint)] focus:outline-none"
          />
          {loading && (
            <Loader2 className="w-4 h-4 animate-spin text-[var(--text-faint)]" />
          )}
          <button
            onClick={onClose}
            aria-label="Close"
            className="p-1 rounded-md text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="max-h-[50vh] overflow-y-auto scrollbar-thin scrollbar-track-transparent">
          {query.trim().length < 2 ? (
            <p className="px-4 py-6 text-sm text-[var(--text-faint)] text-center">
              Type at least 2 characters to search.
            </p>
          ) : !loading && searched && results.length === 0 ? (
            <p className="px-4 py-6 text-sm text-[var(--text-faint)] text-center">
              No messages found.
            </p>
          ) : (
            results.map((r, i) => (
              <button
                key={r.message_id ?? i}
                onClick={() => pick(r)}
                className="w-full text-left px-4 py-2.5 flex gap-3 hover:bg-[var(--surface-2)] border-b border-[var(--border-soft)] transition-colors"
              >
                <div
                  className="flex items-center justify-center w-8 h-8 rounded-full text-[11px] font-semibold text-white shrink-0"
                  style={{ background: avatarColor(r.username) }}
                >
                  {initials(r.username)}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 text-[11px] text-[var(--text-faint)]">
                    <Hash className="w-3 h-3" />
                    <span className="truncate">{r.room_name}</span>
                    <span>·</span>
                    <span className="font-medium text-[var(--text-dim)] truncate">
                      {r.username}
                    </span>
                    <span>·</span>
                    <span className="shrink-0">
                      {formatSearchTime(r.created_at)}
                    </span>
                  </div>
                  <div className="text-sm text-[var(--text-dim)] truncate">
                    {r.message}
                  </div>
                </div>
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  );
};
