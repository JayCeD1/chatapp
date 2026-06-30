import React, { useEffect, useMemo } from "react";
import { Hash, RefreshCw, WifiOff, X } from "lucide-react";
import { ChatRoom, Department, Message, User } from "../types";
import { ConnectionStatus } from "../hooks/useChatConnection";
import { Theme } from "../hooks/useTheme";
import { Sidebar } from "./Sidebar";
import { ChatPane } from "./ChatPane";
import { MembersPanel, Member } from "./MembersPanel";

interface WorkspaceProps {
  departments: Department[];
  chatRooms: ChatRoom[];
  currentRoom: ChatRoom | null;
  currentUser: User;
  messages: Message[];
  loadingMessages: boolean;
  membersByRoom: Record<string, string[]>;
  connectionStatus: ConnectionStatus;
  error: string | null;
  hasMore: boolean;
  onSelectRoom: (room: ChatRoom) => void;
  onSendMessage: (text: string, isEmoji?: boolean) => void;
  onLoadOlder: () => Promise<void>;
  onLeaveRoom: () => void;
  onLogout: () => void;
  onDismissError: () => void;
  theme: Theme;
  onToggleTheme: () => void;
}

export const Workspace: React.FC<WorkspaceProps> = ({
  departments,
  chatRooms,
  currentRoom,
  currentUser,
  messages,
  loadingMessages,
  membersByRoom,
  connectionStatus,
  error,
  hasMore,
  onSelectRoom,
  onSendMessage,
  onLoadOlder,
  onLeaveRoom,
  onLogout,
  onDismissError,
  theme,
  onToggleTheme,
}) => {
  // Live roster for the active room (server truth via UserList). Everyone in it is
  // connected; ensure the current user shows even before the first roster arrives.
  const members: Member[] = useMemo(() => {
    if (!currentRoom) return [];
    const names = new Set<string>(membersByRoom[currentRoom.name] || []);
    names.add(currentUser.name);
    return Array.from(names)
      .map((name) => ({
        name,
        online: true,
        isYou: name === currentUser.name,
      }))
      .sort((a, b) =>
        a.isYou ? -1 : b.isYou ? 1 : a.name.localeCompare(b.name),
      );
  }, [currentRoom, membersByRoom, currentUser.name]);

  const onlineCount = members.length;

  return (
    <div className="h-dvh w-full grid grid-cols-[clamp(220px,22vw,280px)_1fr] lg:grid-cols-[clamp(220px,22vw,280px)_1fr_clamp(200px,18vw,260px)] bg-[var(--bg)] text-[var(--text)] overflow-hidden">
      <Sidebar
        departments={departments}
        chatRooms={chatRooms}
        currentRoom={currentRoom}
        currentUser={currentUser}
        connectionStatus={connectionStatus}
        onSelectRoom={onSelectRoom}
        onLogout={onLogout}
        theme={theme}
        onToggleTheme={onToggleTheme}
      />

      <div className="flex flex-col min-w-0 min-h-0">
        {connectionStatus !== "connected" && (
          <ConnectionBanner status={connectionStatus} />
        )}
        {currentRoom ? (
          <ChatPane
            room={currentRoom}
            currentUser={currentUser}
            messages={messages}
            loading={loadingMessages}
            hasMore={hasMore}
            onlineCount={onlineCount}
            memberCount={members.length}
            onSendMessage={onSendMessage}
            onLoadOlder={onLoadOlder}
            onLeave={onLeaveRoom}
          />
        ) : (
          <EmptyState />
        )}
      </div>

      <MembersPanel members={members} />

      {error && <ErrorToast message={error} onClose={onDismissError} />}
    </div>
  );
};

const ConnectionBanner: React.FC<{ status: ConnectionStatus }> = ({
  status,
}) => {
  const reconnecting = status === "reconnecting";
  return (
    <div
      role="status"
      className={`flex items-center justify-center gap-2 py-1.5 text-sm font-medium ${
        reconnecting
          ? "bg-[#d29922]/15 text-[#e3b341]"
          : "bg-[var(--danger)]/15 text-[var(--danger)]"
      }`}
    >
      {reconnecting ? (
        <RefreshCw className="w-4 h-4 animate-spin" />
      ) : (
        <WifiOff className="w-4 h-4" />
      )}
      {reconnecting
        ? "Connection lost — reconnecting…"
        : "Disconnected. Check the host and your network."}
    </div>
  );
};

const EmptyState: React.FC = () => (
  <div className="flex flex-col items-center justify-center h-full text-center px-6">
    <div className="flex items-center justify-center w-16 h-16 rounded-2xl bg-[var(--surface-2)] mb-4">
      <Hash className="w-8 h-8 text-[var(--accent-strong)]" />
    </div>
    <h2 className="text-xl font-semibold text-[var(--text)]">Pick a channel</h2>
    <p className="text-sm text-[var(--text-dim)] mt-1 max-w-sm">
      Choose a channel from the sidebar to start collaborating with your team.
    </p>
  </div>
);

const ErrorToast: React.FC<{ message: string; onClose: () => void }> = ({
  message,
  onClose,
}) => {
  useEffect(() => {
    const t = setTimeout(onClose, 6000);
    return () => clearTimeout(t);
  }, [message, onClose]);

  return (
    <div
      role="alert"
      className="fixed bottom-5 right-5 z-50 max-w-sm flex items-start gap-3 bg-[var(--surface-2)] border border-[var(--danger)]/40 rounded-xl shadow-2xl px-4 py-3 animate-slide-up"
    >
      <span className="text-sm text-[var(--text)] flex-1">{message}</span>
      <button
        onClick={onClose}
        aria-label="Dismiss"
        className="text-[var(--text-faint)] hover:text-[var(--text)] transition-colors"
      >
        <X className="w-4 h-4" />
      </button>
    </div>
  );
};
