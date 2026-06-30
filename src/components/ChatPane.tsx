import React, { useState, useEffect, useRef } from "react";
import { Send, Smile, Hash, LogOut, Loader2 } from "lucide-react";
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
  messages: Message[];
  loading: boolean;
  onlineCount: number;
  memberCount: number;
  onSendMessage: (text: string, isEmoji?: boolean) => void;
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
  messages,
  loading,
  onlineCount,
  memberCount,
  onSendMessage,
  onLeave,
}) => {
  const [inputText, setInputText] = useState("");
  const [showEmoji, setShowEmoji] = useState(false);
  const endRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [atBottom, setAtBottom] = useState(true);

  // Only auto-scroll when the user is already near the bottom, so reading history
  // isn't yanked away by an incoming message.
  useEffect(() => {
    if (atBottom) endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, atBottom]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const near = el.scrollHeight - el.scrollTop - el.clientHeight < 120;
    setAtBottom(near);
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
      <div
        ref={scrollRef}
        onScroll={onScroll}
        role="log"
        aria-live="polite"
        aria-label={`Messages in ${room.name}`}
        className="flex-1 min-h-0 overflow-y-auto px-4 py-4 scrollbar-thin scrollbar-thumb-white/10 scrollbar-track-transparent"
      >
        {loading ? (
          <div className="flex items-center justify-center h-full text-[var(--text-faint)] gap-2">
            <Loader2 className="w-5 h-5 animate-spin" />
            Loading messages…
          </div>
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
          messages.map((msg, idx) => {
            const prev = messages[idx - 1];
            const showDate = !prev || !sameDay(prev.created_at, msg.created_at);

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
            const isMe = msg.username === currentUser.name;

            return (
              <React.Fragment key={msg.message_id ?? msg.id ?? idx}>
                {showDate && <DateSeparator iso={msg.created_at} />}
                <div
                  className={`group flex gap-3 px-2 ${
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
                    <div
                      className={`text-[var(--text-dim)] break-words leading-relaxed ${
                        msg.is_emoji ? "text-3xl" : "text-sm"
                      }`}
                    >
                      {msg.message}
                    </div>
                  </div>
                </div>
              </React.Fragment>
            );
          })
        )}
        <div ref={endRef} />
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
