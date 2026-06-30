import React, { useState, useEffect, useLayoutEffect, useRef } from "react";
import {
  Send,
  Smile,
  SmilePlus,
  Hash,
  Lock,
  MessageSquare,
  UserPlus,
  LogOut,
  ChevronDown,
  Loader2,
  Pencil,
  Trash2,
  Check,
  X,
} from "lucide-react";
import { ChatRoom, DirectoryUser, Message, Reaction, User } from "../types";
import { InviteModal } from "./InviteModal";
import {
  initials,
  avatarColor,
  formatTime,
  formatDateSeparator,
  sameDay,
  shouldGroup,
  isSystem,
  parseMentions,
} from "../utils";

interface ChatPaneProps {
  room: ChatRoom;
  currentUser: User;
  currentUserId: number;
  // Host-assigned canonical id (client mode); our local id differs, so we match own messages
  // by either. Null in host mode (currentUserId is already canonical there).
  canonicalUserId: number | null;
  messages: Message[];
  loading: boolean;
  hasMore: boolean;
  onlineCount: number;
  memberCount: number;
  typingUsers: string[];
  onTyping: (typing: boolean) => void;
  onSendMessage: (text: string, isEmoji?: boolean) => void;
  onEditMessage: (targetId: string, newText: string) => Promise<void>;
  onDeleteMessage: (targetId: string) => Promise<void>;
  reactions: Record<string, Reaction[]>;
  onToggleReaction: (targetId: string, emoji: string) => Promise<void>;
  onLoadOlder: () => Promise<void>;
  onLeave: () => void;
  directory: DirectoryUser[];
  onAddMember: (roomId: number, userId: number) => void;
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
  canonicalUserId,
  messages,
  loading,
  hasMore,
  onlineCount,
  memberCount,
  typingUsers,
  onTyping,
  onSendMessage,
  onEditMessage,
  onDeleteMessage,
  reactions,
  onToggleReaction,
  onLoadOlder,
  onLeave,
  directory,
  onAddMember,
}) => {
  // DMs are stored under a synthetic name; show the derived label and drop the "#" prefix.
  const isDm = !!room.is_dm;
  const title = isDm ? room.display_name || room.name : room.name;
  const prefix = isDm ? "" : "#";
  const [inputText, setInputText] = useState("");
  const [showEmoji, setShowEmoji] = useState(false);
  const [showInvite, setShowInvite] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState("");
  const [reactingId, setReactingId] = useState<string | null>(null);

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

  // Reset per-room view state whenever the open channel changes (ChatPane is reused,
  // not remounted): pin to bottom, close any open editor, release scroll/load guards.
  useEffect(() => {
    setAtBottom(true);
    setEditingId(null);
    setEditText("");
    setReactingId(null);
    restoreRef.current = null;
    loadingOlderRef.current = false;
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
        // Release the scroll anchor even if the page was empty (no [messages] change,
        // so the layout effect never ran) — otherwise auto-scroll stays dead.
        restoreRef.current = null;
      });
    }
  };

  const jumpToLatest = () => {
    setAtBottom(true);
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  };

  // Typing signal: throttle "start" re-sends to once / 2.5s (refreshes the peer's
  // 5s expiry while typing) and fire one "stop" after 3s idle. lastStartRef==0 means
  // "not currently typing".
  const lastStartRef = useRef(0);
  const stopTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const stopTyping = () => {
    if (stopTimerRef.current) {
      clearTimeout(stopTimerRef.current);
      stopTimerRef.current = null;
    }
    if (lastStartRef.current !== 0) {
      lastStartRef.current = 0;
      onTyping(false);
    }
  };

  const signalTyping = () => {
    const now = Date.now();
    if (now - lastStartRef.current > 2500) {
      lastStartRef.current = now;
      onTyping(true);
    }
    if (stopTimerRef.current) clearTimeout(stopTimerRef.current);
    stopTimerRef.current = setTimeout(stopTyping, 3000);
  };

  // Drop any pending stop + reset typing state when the room changes/unmounts; the
  // old room's indicator self-expires on the peer's 5s ticker.
  useEffect(() => {
    return () => {
      if (stopTimerRef.current) clearTimeout(stopTimerRef.current);
      lastStartRef.current = 0;
    };
  }, [room.id]);

  const handleSend = () => {
    const text = inputText.trim();
    if (!text) return;
    onSendMessage(text);
    setInputText("");
    setShowEmoji(false);
    stopTyping();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setInputText(e.target.value);
    if (e.target.value.trim()) signalTyping();
    else stopTyping();
  };

  // "Alice is typing…" / "Alice and Bob…" / "Alice, Bob and 2 others…"
  const typingLabel = (() => {
    const n = typingUsers.length;
    if (n === 0) return "";
    if (n === 1) return `${typingUsers[0]} is typing…`;
    if (n === 2) return `${typingUsers[0]} and ${typingUsers[1]} are typing…`;
    return `${typingUsers[0]}, ${typingUsers[1]} and ${n - 2} other${
      n - 2 > 1 ? "s" : ""
    } are typing…`;
  })();

  return (
    <section className="flex flex-col h-full min-w-0 bg-[var(--bg)]">
      {/* Header */}
      <header className="flex items-center justify-between px-5 h-14 border-b border-[var(--border)] shrink-0">
        <div className="flex items-center gap-2 min-w-0">
          {isDm ? (
            <MessageSquare className="w-4 h-4 text-[var(--text-faint)] shrink-0" />
          ) : room.is_private ? (
            <Lock className="w-4 h-4 text-[var(--text-faint)] shrink-0" />
          ) : (
            <Hash className="w-5 h-5 text-[var(--text-faint)] shrink-0" />
          )}
          <h2 className="font-semibold text-[var(--text)] truncate">{title}</h2>
          {!isDm && (
            <span className="text-[var(--text-faint)] text-sm shrink-0">
              · {memberCount} {memberCount === 1 ? "member" : "members"}
              {onlineCount > 0 && (
                <span className="text-[var(--online)]">
                  {" "}
                  · {onlineCount} online
                </span>
              )}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1 shrink-0">
          {room.is_private && !isDm && (
            <button
              onClick={() => setShowInvite(true)}
              title="Add people"
              aria-label="Add people"
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-sm text-[var(--text-dim)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
            >
              <UserPlus className="w-4 h-4" />
              <span className="hidden sm:inline">Add</span>
            </button>
          )}
          <button
            onClick={onLeave}
            title="Leave channel"
            aria-label="Leave channel"
            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-sm text-[var(--text-dim)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
          >
            <LogOut className="w-4 h-4" />
            <span className="hidden sm:inline">Leave</span>
          </button>
        </div>
      </header>

      {showInvite && (
        <InviteModal
          roomName={room.name}
          users={directory}
          selfId={currentUserId}
          selfName={currentUser.name}
          onAdd={(userId) => onAddMember(room.id, userId)}
          onClose={() => setShowInvite(false)}
        />
      )}

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
                {isDm ? (
                  <MessageSquare className="w-7 h-7 text-[var(--accent-strong)]" />
                ) : (
                  <Hash className="w-7 h-7 text-[var(--accent-strong)]" />
                )}
              </div>
              <h3 className="text-lg font-semibold text-[var(--text)]">
                {isDm ? `Chat with ${title}` : `Welcome to #${title}`}
              </h3>
              <p className="text-sm text-[var(--text-dim)] mt-1 max-w-sm">
                {isDm
                  ? "This is the start of your conversation."
                  : `${room.description || "This is the start of the channel."} Say hello to get things going.`}
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
                  You've reached the beginning of {prefix}
                  {title}
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
                // Own messages match our local id (live, optimistic) OR our canonical id
                // (history / echoes). Matching by id — not name — so same-named users stay
                // distinct.
                const isMe =
                  msg.user_id === currentUserId ||
                  (canonicalUserId != null && msg.user_id === canonicalUserId);
                const isDeleted = !!msg.deleted_at;
                const canModify = isMe && !isDeleted && !!msg.message_id;
                const isEditing =
                  editingId != null && editingId === msg.message_id;
                const msgReactions = msg.message_id
                  ? reactions[msg.message_id] || []
                  : [];

                return (
                  <React.Fragment key={msg.message_id ?? msg.id ?? idx}>
                    {showDate && <DateSeparator iso={msg.created_at} />}
                    <div
                      className={`group relative flex gap-3 px-2 ${
                        grouped ? "mt-0.5" : "mt-3"
                      } py-0.5 rounded-md hover:bg-[var(--surface)]/60 ${
                        isMe ? "flex-row-reverse" : ""
                      }`}
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
                      <div
                        className={`min-w-0 flex-1 flex flex-col ${
                          isMe ? "items-end" : "items-start"
                        }`}
                      >
                        {!grouped && (
                          <div
                            className={`flex items-baseline gap-2 ${
                              isMe ? "flex-row-reverse" : ""
                            }`}
                          >
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
                            className={`break-words leading-relaxed max-w-[90%] ${
                              msg.is_emoji ? "text-3xl" : "text-sm"
                            } ${
                              isMe && !msg.is_emoji
                                ? "bg-[var(--accent-soft)] text-[var(--text)] rounded-2xl px-3 py-1.5"
                                : "text-[var(--text-dim)]"
                            } ${isMe ? "text-right" : ""}`}
                          >
                            <MessageText
                              text={msg.message}
                              meName={currentUser.name}
                            />
                            {msg.edited_at && (
                              <span className="text-[11px] text-[var(--text-faint)] ml-1">
                                (edited)
                              </span>
                            )}
                          </div>
                        )}

                        {msgReactions.length > 0 && (
                          <div
                            className={`flex flex-wrap gap-1 mt-1 ${
                              isMe ? "justify-end" : ""
                            }`}
                          >
                            {msgReactions.map((r) => (
                              <button
                                key={r.emoji}
                                onClick={() =>
                                  msg.message_id &&
                                  onToggleReaction(msg.message_id, r.emoji)
                                }
                                className={`flex items-center gap-1 px-1.5 py-0.5 rounded-full text-xs border transition-colors ${
                                  r.me
                                    ? "bg-[var(--accent-soft)] border-[var(--accent)] text-[var(--text)]"
                                    : "bg-[var(--surface-2)] border-[var(--border)] text-[var(--text-dim)] hover:border-[var(--text-faint)]"
                                }`}
                                title={
                                  r.me ? "Click to remove" : "Click to add"
                                }
                              >
                                <span>{r.emoji}</span>
                                <span>{r.count}</span>
                              </button>
                            ))}
                          </div>
                        )}
                      </div>

                      {!isEditing && msg.message_id && !isDeleted && (
                        <div
                          className={`absolute top-0 hidden group-hover:flex items-center gap-0.5 bg-[var(--surface)] border border-[var(--border)] rounded-md shadow-sm ${
                            isMe ? "left-2" : "right-2"
                          }`}
                        >
                          <button
                            onClick={() =>
                              setReactingId(
                                reactingId === msg.message_id
                                  ? null
                                  : msg.message_id!,
                              )
                            }
                            title="Add reaction"
                            aria-label="Add reaction"
                            className="p-1.5 rounded-md text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)]"
                          >
                            <SmilePlus className="w-3.5 h-3.5" />
                          </button>
                          {canModify && (
                            <>
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
                            </>
                          )}
                        </div>
                      )}

                      {reactingId === msg.message_id && msg.message_id && (
                        <div
                          className={`absolute top-8 z-50 bg-[var(--surface-2)] border border-[var(--border)] rounded-xl shadow-2xl p-2 grid grid-cols-6 gap-1 animate-scale-in ${
                            isMe ? "left-2" : "right-2"
                          }`}
                        >
                          {EMOJIS.map((e) => (
                            <button
                              key={e}
                              onClick={() => {
                                onToggleReaction(msg.message_id!, e);
                                setReactingId(null);
                              }}
                              className="text-lg hover:bg-[var(--surface-3)] p-1 rounded-lg transition-colors"
                              aria-label={`React ${e}`}
                            >
                              {e}
                            </button>
                          ))}
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
        <div
          className="h-4 px-1 mb-0.5 text-xs text-[var(--text-faint)] truncate"
          aria-live="polite"
        >
          {typingLabel && (
            <span className="inline-flex items-center gap-1.5 animate-fade-in">
              <span className="flex gap-0.5">
                <span className="w-1 h-1 rounded-full bg-[var(--text-faint)] animate-typing-dot" />
                <span className="w-1 h-1 rounded-full bg-[var(--text-faint)] animate-typing-dot [animation-delay:150ms]" />
                <span className="w-1 h-1 rounded-full bg-[var(--text-faint)] animate-typing-dot [animation-delay:300ms]" />
              </span>
              {typingLabel}
            </span>
          )}
        </div>
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
              Message {prefix}
              {title}
            </label>
            <input
              id="composer"
              type="text"
              value={inputText}
              onChange={handleInputChange}
              onKeyDown={handleKeyDown}
              placeholder={`Message ${prefix}${title}`}
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

// Render message text with @mentions highlighted (extra emphasis if it's you).
const MessageText: React.FC<{ text: string; meName: string }> = ({
  text,
  meName,
}) => (
  <>
    {parseMentions(text).map((part, i) =>
      part.mention ? (
        <span
          key={i}
          className={`rounded px-0.5 font-medium ${
            part.text.slice(1).toLowerCase() === meName.toLowerCase()
              ? "bg-[var(--accent-soft)] text-[var(--accent-strong)]"
              : "text-[var(--accent-strong)]"
          }`}
        >
          {part.text}
        </span>
      ) : (
        <React.Fragment key={i}>{part.text}</React.Fragment>
      ),
    )}
  </>
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
