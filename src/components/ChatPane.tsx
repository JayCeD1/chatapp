import React, { useState, useEffect, useLayoutEffect, useRef } from "react";
import {
  Send,
  Smile,
  Hash,
  LogOut,
  ChevronDown,
  Loader2,
  Pencil,
  Trash2,
  Check,
  X,
} from "lucide-react";
import { ChatRoom, Message, User } from "../types";
import {
  initials,
  avatarColor,
  formatTime,
  formatDateSeparator,
  sameDay,
  shouldGroup,
  isSystem,
} from "../utils";

interface ChatPaneProps {
  room: ChatRoom;
  currentUser: User;
  currentUserId: number;
  messages: Message[];
  loading: boolean;
  hasMore: boolean;
  onlineCount: number;
  memberCount: number;
  onSendMessage: (text: string, isEmoji?: boolean) => void;
  onEditMessage: (targetId: string, newText: string) => Promise<void>;
  onDeleteMessage: (targetId: string) => Promise<void>;
  onLoadOlder: () => Promise<void>;
  onLeave: () => void;
}

const EMOJIS = [
  "😊",
  "🤔",
  "😂",
  "🚀",
  "👍",
  "👎",
  "❤️",
  "🎉",
  "🔥",
  "💯",
  "👀",
  "🙌",
];

export const ChatPane: React.FC<ChatPaneProps> = ({
  room,
  currentUser,
  currentUserId,
  messages,
  loading,
  hasMore,
  onlineCount,
  memberCount,
  onSendMessage,
  onEditMessage,
  onDeleteMessage,
  onLoadOlder,
  onLeave,
}) => {
  const [inputText, setInputText] = useState("");
  const [showEmoji, setShowEmoji] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState("");

  const startEdit = (msg: Message) => {
    if (!msg.message_id) return;
    setEditingId(msg.message_id);
    setEditText(msg.message);
  };
  const cancelEdit = () => {
    setEditingId(null);
    setEditText("");
  };
  const saveEdit = async (targetId: string) => {
    const text = editText.trim();
    cancelEdit();
    if (text) await onEditMessage(targetId, text);
  };
  const confirmDelete = (msg: Message) => {
    if (!msg.message_id) return;
    if (window.confirm("Delete this message?")) onDeleteMessage(msg.message_id);
  };
  const endRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [atBottom, setAtBottom] = useState(true);
  // While loading older history, remember the scroll metrics so we can keep the
  // viewport anchored after the prepended messages change the scroll height.
  const restoreRef = useRef<{ height: number; top: number } | null>(null);
  const loadingOlderRef = useRef(false);

  // Only auto-scroll when the user is already near the bottom, so reading history
  // isn't yanked away by an incoming message.
  useEffect(() => {
    if (restoreRef.current) return; // a prepend is being anchored, don't jump
    if (atBottom) endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, atBottom]);

  // After older messages prepend, restore the viewport so it doesn't jump to the top.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (restoreRef.current && el) {
      el.scrollTop =
        el.scrollHeight - restoreRef.current.height + restoreRef.current.top;
      restoreRef.current = null;
    }
  }, [messages]);

  // Start pinned to the bottom whenever the open channel changes.
  useEffect(() => {
    setAtBottom(true);
  }, [room.id]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const near = el.scrollHeight - el.scrollTop - el.clientHeight < 120;
    setAtBottom(near);

    // Near the top → pull in older history (once), anchoring the viewport.
    if (el.scrollTop < 80 && hasMore && !loadingOlderRef.current && !loading) {
      loadingOlderRef.current = true;
      restoreRef.current = { height: el.scrollHeight, top: el.scrollTop };
      setLoadingOlder(true);
      onLoadOlder().finally(() => {
        loadingOlderRef.current = false;
        setLoadingOlder(false);
      });
    }
  };

  const jumpToLatest = () => {
    setAtBottom(true);
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  };

  const handleSend = () => {
    const text = inputText.trim();
    if (!text) return;
    onSendMessage(text);
    setInputText("");
    setShowEmoji(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <section className="flex flex-col h-full min-w-0 bg-[var(--bg)]">
      {/* Header */}
      <header className="flex items-center justify-between px-5 h-14 border-b border-[var(--border)] shrink-0">
        <div className="flex items-center gap-2 min-w-0">
          <Hash className="w-5 h-5 text-[var(--text-faint)] shrink-0" />
          <h2 className="font-semibold text-[var(--text)] truncate">
            {room.name}
          </h2>
          <span className="text-[var(--text-faint)] text-sm shrink-0">
            · {memberCount} {memberCount === 1 ? "member" : "members"}
            {onlineCount > 0 && (
              <span className="text-[var(--online)]">
                {" "}
                · {onlineCount} online
              </span>
            )}
          </span>
        </div>
        <button
          onClick={onLeave}
          title="Leave channel"
          aria-label="Leave channel"
          className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-sm text-[var(--text-dim)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
        >
          <LogOut className="w-4 h-4" />
          <span className="hidden sm:inline">Leave</span>
        </button>
      </header>

      {/* Messages */}
      <div className="relative flex-1 min-h-0">
        <div
          ref={scrollRef}
          onScroll={onScroll}
          role="log"
          aria-live="polite"
          aria-label={`Messages in ${room.name}`}
          className="absolute inset-0 overflow-y-auto px-4 py-4 scrollbar-thin scrollbar-track-transparent"
        >
          {loading ? (
            <MessageSkeletons />
          ) : messages.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full text-center px-6">
              <div className="flex items-center justify-center w-14 h-14 rounded-2xl bg-[var(--surface-2)] mb-4">
                <Hash className="w-7 h-7 text-[var(--accent-strong)]" />
              </div>
              <h3 className="text-lg font-semibold text-[var(--text)]">
                Welcome to #{room.name}
              </h3>
              <p className="text-sm text-[var(--text-dim)] mt-1 max-w-sm">
                {room.description || "This is the start of the channel."} Say
                hello to get things going.
              </p>
            </div>
          ) : (
            <>
              {loadingOlder ? (
                <div className="flex justify-center py-2">
                  <Loader2 className="w-4 h-4 animate-spin text-[var(--text-faint)]" />
                </div>
              ) : !hasMore ? (
                <div className="text-center py-3 text-[11px] text-[var(--text-faint)]">
                  You've reached the beginning of #{room.name}
                </div>
              ) : null}
              {messages.map((msg, idx) => {
                const prev = messages[idx - 1];
                const showDate =
                  !prev || !sameDay(prev.created_at, msg.created_at);

                if (isSystem(msg)) {
                  return (
                    <React.Fragment key={msg.message_id ?? msg.id ?? idx}>
                      {showDate && <DateSeparator iso={msg.created_at} />}
                      <div className="flex justify-center my-1.5">
                        <span className="text-[12px] text-[var(--text-faint)] bg-[var(--surface)] px-3 py-1 rounded-full">
                          {msg.message}
                        </span>
                      </div>
                    </React.Fragment>
                  );
                }

                const grouped = !showDate && shouldGroup(prev, msg);
                const isMe =
                  msg.user_id === currentUserId ||
                  msg.username === currentUser.name;
                const isDeleted = !!msg.deleted_at;
                const canModify = isMe && !isDeleted && !!msg.message_id;
                const isEditing =
                  editingId != null && editingId === msg.message_id;

                return (
                  <React.Fragment key={msg.message_id ?? msg.id ?? idx}>
                    {showDate && <DateSeparator iso={msg.created_at} />}
                    <div
                      className={`group relative flex gap-3 px-2 ${
                        grouped ? "mt-0.5" : "mt-3"
                      } py-0.5 rounded-md hover:bg-[var(--surface)]/60`}
                    >
                      {grouped ? (
                        <div className="w-9 shrink-0 text-[10px] text-[var(--text-faint)] text-right pr-1 pt-1 opacity-0 group-hover:opacity-100">
                          {formatTime(msg.created_at)}
                        </div>
                      ) : (
                        <div
                          className="flex items-center justify-center w-9 h-9 rounded-full text-xs font-semibold text-white shrink-0"
                          style={{ background: avatarColor(msg.username) }}
                        >
                          {initials(msg.username)}
                        </div>
                      )}
                      <div className="min-w-0 flex-1">
                        {!grouped && (
                          <div className="flex items-baseline gap-2">
                            <span
                              className={`text-sm font-semibold ${
                                isMe
                                  ? "text-[var(--accent-strong)]"
                                  : "text-[var(--text)]"
                              }`}
                            >
                              {msg.username}
                              {isMe && (
                                <span className="text-[var(--text-faint)] font-normal">
                                  {" "}
                                  (you)
                                </span>
                              )}
                            </span>
                            <span className="text-[11px] text-[var(--text-faint)]">
                              {formatTime(msg.created_at)}
                            </span>
                          </div>
                        )}

                        {isEditing ? (
                          <div className="flex items-center gap-1.5 mt-0.5">
                            <input
                              value={editText}
                              onChange={(e) => setEditText(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === "Enter") {
                                  e.preventDefault();
                                  saveEdit(msg.message_id!);
                                } else if (e.key === "Escape") {
                                  cancelEdit();
                                }
                              }}
                              autoFocus
                              className="flex-1 bg-[var(--surface-2)] border border-[var(--border)] rounded-lg px-3 py-1.5 text-sm text-[var(--text)] focus:outline-none focus:border-[var(--accent)]"
                            />
                            <button
                              onClick={() => saveEdit(msg.message_id!)}
                              aria-label="Save edit"
                              className="p-1.5 rounded-md text-[var(--online)] hover:bg-[var(--surface-2)]"
                            >
                              <Check className="w-4 h-4" />
                            </button>
                            <button
                              onClick={cancelEdit}
                              aria-label="Cancel edit"
                              className="p-1.5 rounded-md text-[var(--text-faint)] hover:bg-[var(--surface-2)]"
                            >
                              <X className="w-4 h-4" />
                            </button>
                          </div>
                        ) : isDeleted ? (
                          <div className="text-sm italic text-[var(--text-faint)]">
                            This message was deleted
                          </div>
                        ) : (
                          <div
                            className={`text-[var(--text-dim)] break-words leading-relaxed ${
                              msg.is_emoji ? "text-3xl" : "text-sm"
                            }`}
                          >
                            {msg.message}
                            {msg.edited_at && (
                              <span className="text-[11px] text-[var(--text-faint)] ml-1">
                                (edited)
                              </span>
                            )}
                          </div>
                        )}
                      </div>

                      {canModify && !isEditing && (
                        <div className="absolute top-0 right-2 hidden group-hover:flex items-center gap-0.5 bg-[var(--surface)] border border-[var(--border)] rounded-md shadow-sm">
                          <button
                            onClick={() => startEdit(msg)}
                            title="Edit"
                            aria-label="Edit message"
                            className="p-1.5 rounded-md text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)]"
                          >
                            <Pencil className="w-3.5 h-3.5" />
                          </button>
                          <button
                            onClick={() => confirmDelete(msg)}
                            title="Delete"
                            aria-label="Delete message"
                            className="p-1.5 rounded-md text-[var(--text-faint)] hover:text-[var(--danger)] hover:bg-[var(--surface-2)]"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </div>
                      )}
                    </div>
                  </React.Fragment>
                );
              })}
            </>
          )}
          <div ref={endRef} />
        </div>

        {!loading && !atBottom && messages.length > 0 && (
          <button
            onClick={jumpToLatest}
            aria-label="Jump to latest messages"
            className="absolute bottom-3 left-1/2 -translate-x-1/2 flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-[var(--accent)] text-white text-xs font-medium shadow-lg hover:bg-[var(--accent-strong)] transition-colors animate-fade-in"
          >
            <ChevronDown className="w-4 h-4" /> Jump to latest
          </button>
        )}
      </div>

      {/* Composer */}
      <div className="px-4 pb-4 pt-2 shrink-0">
        <div className="relative flex items-end gap-2">
          {showEmoji && (
            <div className="absolute bottom-14 left-0 bg-[var(--surface-2)] border border-[var(--border)] p-2 rounded-xl shadow-2xl grid grid-cols-6 gap-1 z-50 animate-scale-in">
              {EMOJIS.map((emoji) => (
                <button
                  key={emoji}
                  onClick={() => setInputText((p) => p + emoji)}
                  className="text-xl hover:bg-[var(--surface-3)] p-1.5 rounded-lg transition-colors"
                  aria-label={`Insert ${emoji}`}
                >
                  {emoji}
                </button>
              ))}
            </div>
          )}

          <button
            onClick={() => setShowEmoji((s) => !s)}
            aria-label="Emoji picker"
            className={`p-2.5 rounded-lg transition-colors ${
              showEmoji
                ? "bg-[var(--surface-3)] text-[var(--text)]"
                : "text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)]"
            }`}
          >
            <Smile className="w-5 h-5" />
          </button>

          <div className="flex-1 bg-[var(--surface-2)] border border-[var(--border)] rounded-xl focus-within:border-[var(--accent)] transition-colors flex items-center">
            <label htmlFor="composer" className="sr-only">
              Message #{room.name}
            </label>
            <input
              id="composer"
              type="text"
              value={inputText}
              onChange={(e) => setInputText(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={`Message #${room.name}`}
              autoComplete="off"
              className="w-full bg-transparent border-none text-[var(--text)] placeholder-[var(--text-faint)] px-4 py-3 focus:outline-none"
            />
          </div>

          <button
            onClick={handleSend}
            disabled={!inputText.trim()}
            aria-label="Send message"
            className="p-2.5 rounded-lg bg-[var(--accent)] text-white shadow-lg shadow-[var(--accent-soft)] hover:bg-[var(--accent-strong)] disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
          >
            <Send className="w-5 h-5" />
          </button>
        </div>
      </div>
    </section>
  );
};

const DateSeparator: React.FC<{ iso: string }> = ({ iso }) => (
  <div className="flex items-center gap-3 my-3 px-2" role="separator">
    <div className="flex-1 h-px bg-[var(--border)]" />
    <span className="text-[11px] font-medium text-[var(--text-faint)]">
      {formatDateSeparator(iso)}
    </span>
    <div className="flex-1 h-px bg-[var(--border)]" />
  </div>
);

const MessageSkeletons: React.FC = () => (
  <div className="space-y-4 px-2 pt-2" aria-hidden="true">
    {Array.from({ length: 6 }).map((_, i) => (
      <div key={i} className="flex gap-3">
        <div className="skeleton w-9 h-9 rounded-full shrink-0" />
        <div className="flex-1 space-y-2 pt-1">
          <div
            className="skeleton h-3"
            style={{ width: `${28 + ((i * 17) % 28)}%` }}
          />
          <div
            className="skeleton h-3"
            style={{ width: `${50 + ((i * 23) % 38)}%` }}
          />
        </div>
      </div>
    ))}
  </div>
);
