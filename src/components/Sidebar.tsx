import React, { useState } from "react";
import {
  Hash,
  LogOut,
  Users,
  Sun,
  Moon,
  Plus,
  Search,
  MessageSquare,
  Settings,
} from "lucide-react";
import {
  ChatRoom,
  Department,
  DirectoryUser,
  SearchResult,
  User,
} from "../types";
import { ConnectionStatus } from "../hooks/useChatConnection";
import { Theme } from "../hooks/useTheme";
import { Preferences } from "../preferences";
import { initials, avatarColor } from "../utils";
import { CreateChannelModal } from "./CreateChannelModal";
import { SearchModal } from "./SearchModal";
import { NewDmModal } from "./NewDmModal";
import { SettingsModal } from "./SettingsModal";

interface SidebarProps {
  departments: Department[];
  chatRooms: ChatRoom[];
  currentRoom: ChatRoom | null;
  currentUser: User;
  unreadByRoom: Record<number, number>;
  connectionStatus: ConnectionStatus;
  directory: DirectoryUser[];
  onSelectRoom: (room: ChatRoom) => void;
  onCreateRoom: (
    name: string,
    description: string,
    departmentId: number | null,
    isPrivate: boolean,
  ) => Promise<void>;
  onCreateDm: (targetIds: number[]) => Promise<void> | void;
  onSearch: (query: string) => Promise<SearchResult[]>;
  onJumpToRoom: (roomId: number) => void;
  onLogout: () => void;
  theme: Theme;
  onToggleTheme: () => void;
  preferences: Preferences;
  onSetPreferences: (patch: Partial<Preferences>) => void;
}

const statusMeta: Record<ConnectionStatus, { color: string; label: string }> = {
  connected: { color: "var(--online)", label: "Online" },
  reconnecting: { color: "#d29922", label: "Reconnecting…" },
  disconnected: { color: "var(--danger)", label: "Disconnected" },
};

export const Sidebar: React.FC<SidebarProps> = ({
  departments,
  chatRooms,
  currentRoom,
  currentUser,
  unreadByRoom,
  connectionStatus,
  directory,
  onSelectRoom,
  onCreateRoom,
  onCreateDm,
  onSearch,
  onJumpToRoom,
  onLogout,
  theme,
  onToggleTheme,
  preferences,
  onSetPreferences,
}) => {
  const [showCreate, setShowCreate] = useState(false);
  const [showSearch, setShowSearch] = useState(false);
  const [showNewDm, setShowNewDm] = useState(false);
  const [showSettings, setShowSettings] = useState(false);

  // DMs live in their own section; channels are grouped by department.
  const channels = chatRooms.filter((r) => !r.is_dm);
  const dms = chatRooms.filter((r) => r.is_dm);

  // Group channels by department; keep any unmatched rooms under "Other".
  const groups = departments
    .map((dep) => ({
      name: dep.name,
      rooms: channels.filter((r) => r.department_name === dep.name),
    }))
    .filter((g) => g.rooms.length > 0);

  const matched = new Set(groups.flatMap((g) => g.rooms.map((r) => r.id)));
  const orphans = channels.filter((r) => !matched.has(r.id));
  if (orphans.length) groups.push({ name: "Other", rooms: orphans });

  const status = statusMeta[connectionStatus];

  // One sidebar row. `Icon` differs for channels (#) vs DMs (message bubble); `label` lets
  // DMs show their derived display name rather than the synthetic stored name.
  const roomRow = (
    room: ChatRoom,
    label: string,
    Icon: typeof Hash,
    showCount: boolean,
  ) => {
    const active = currentRoom?.id === room.id;
    // The active room is always read; don't badge what you're looking at.
    const unread = active ? 0 : (unreadByRoom[room.id] ?? 0);
    return (
      <li key={room.id}>
        <button
          onClick={() => onSelectRoom(room)}
          aria-current={active ? "true" : undefined}
          className={`group w-full flex items-center gap-2 px-2.5 py-1.5 rounded-md text-sm transition-colors ${
            active
              ? "bg-[var(--surface-3)] text-[var(--text)]"
              : unread > 0
                ? "text-[var(--text)] hover:bg-[var(--surface-2)]"
                : "text-[var(--text-dim)] hover:bg-[var(--surface-2)] hover:text-[var(--text)]"
          }`}
        >
          <Icon
            className={`w-4 h-4 shrink-0 ${
              active
                ? "text-[var(--accent-strong)]"
                : "text-[var(--text-faint)]"
            }`}
          />
          <span
            className={`truncate flex-1 text-left ${
              unread > 0 ? "font-semibold" : ""
            }`}
          >
            {label}
          </span>
          {unread > 0 ? (
            <span
              className="min-w-[18px] h-[18px] px-1.5 flex items-center justify-center rounded-full bg-[var(--accent)] text-white text-[10px] font-bold shrink-0"
              aria-label={`${unread} unread message${unread === 1 ? "" : "s"}`}
            >
              {unread > 99 ? "99+" : unread}
            </span>
          ) : (
            showCount &&
            (room.user_count ?? 0) > 0 && (
              <span className="flex items-center gap-1 text-[11px] text-[var(--text-faint)]">
                <Users className="w-3 h-3" />
                {room.user_count}
              </span>
            )
          )}
        </button>
      </li>
    );
  };

  return (
    <aside className="flex h-full flex-col bg-[var(--surface)] border-r border-[var(--border)]">
      {/* Workspace header */}
      <div className="flex items-center gap-2.5 px-4 h-14 border-b border-[var(--border)] shrink-0">
        <div className="flex items-center justify-center w-7 h-7 rounded-lg bg-gradient-to-br from-[var(--accent)] to-[var(--accent-strong)]">
          <Hash className="w-4 h-4 text-white" />
        </div>
        <span className="font-semibold text-[var(--text)] tracking-tight flex-1">
          Nutler
        </span>
        <button
          onClick={() => setShowSearch(true)}
          title="Search messages"
          aria-label="Search messages"
          className="p-1.5 rounded-md text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
        >
          <Search className="w-4 h-4" />
        </button>
        <button
          onClick={() => setShowCreate(true)}
          title="Create a channel"
          aria-label="Create a channel"
          className="p-1.5 rounded-md text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
        >
          <Plus className="w-4 h-4" />
        </button>
      </div>

      {/* Channel list */}
      <nav
        className="flex-1 min-h-0 overflow-y-auto py-3 scrollbar-thin scrollbar-thumb-white/10 scrollbar-track-transparent"
        aria-label="Channels"
      >
        {groups.map((group) => (
          <div key={group.name} className="mb-4">
            <div className="px-4 mb-1 flex items-center justify-between">
              <span className="text-[11px] font-semibold uppercase tracking-wider text-[var(--text-faint)]">
                {group.name}
              </span>
              <span className="text-[11px] text-[var(--text-faint)]">
                {group.rooms.length}
              </span>
            </div>
            <ul className="px-2 space-y-0.5">
              {group.rooms.map((room) => roomRow(room, room.name, Hash, true))}
            </ul>
          </div>
        ))}

        {/* Direct messages */}
        <div className="mb-4">
          <div className="px-4 mb-1 flex items-center justify-between">
            <span className="text-[11px] font-semibold uppercase tracking-wider text-[var(--text-faint)]">
              Direct Messages
            </span>
            <button
              onClick={() => setShowNewDm(true)}
              title="New message"
              aria-label="New message"
              className="p-0.5 rounded text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
            >
              <Plus className="w-3.5 h-3.5" />
            </button>
          </div>
          {dms.length === 0 ? (
            <p className="px-4 py-1 text-[11px] text-[var(--text-faint)]">
              No conversations yet.
            </p>
          ) : (
            <ul className="px-2 space-y-0.5">
              {dms.map((room) =>
                roomRow(
                  room,
                  room.display_name || room.name,
                  MessageSquare,
                  false,
                ),
              )}
            </ul>
          )}
        </div>
      </nav>

      {/* Current user footer */}
      <div className="flex items-center gap-2.5 px-3 h-14 border-t border-[var(--border)] shrink-0">
        <div className="relative shrink-0">
          <div
            className="flex items-center justify-center w-8 h-8 rounded-full text-xs font-semibold text-white"
            style={{ background: avatarColor(currentUser.name) }}
          >
            {initials(currentUser.name)}
          </div>
          <span
            className="absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full border-2 border-[var(--surface)]"
            style={{ background: status.color }}
            title={status.label}
          />
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-sm font-medium text-[var(--text)] truncate">
            {currentUser.name}
          </div>
          <div className="text-[11px] text-[var(--text-faint)] truncate">
            {status.label}
          </div>
        </div>
        <button
          onClick={onToggleTheme}
          title={
            theme === "dark" ? "Switch to light mode" : "Switch to dark mode"
          }
          aria-label={
            theme === "dark" ? "Switch to light mode" : "Switch to dark mode"
          }
          className="p-2 rounded-md text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
        >
          {theme === "dark" ? (
            <Sun className="w-4 h-4" />
          ) : (
            <Moon className="w-4 h-4" />
          )}
        </button>
        <button
          onClick={() => setShowSettings(true)}
          title="Settings"
          aria-label="Settings"
          className="p-2 rounded-md text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
        >
          <Settings className="w-4 h-4" />
        </button>
        <button
          onClick={onLogout}
          title="Log out"
          aria-label="Log out"
          className="p-2 rounded-md text-[var(--text-faint)] hover:text-[var(--danger)] hover:bg-[var(--surface-2)] transition-colors"
        >
          <LogOut className="w-4 h-4" />
        </button>
      </div>

      {showCreate && (
        <CreateChannelModal
          departments={departments}
          defaultDepartmentId={
            currentRoom?.department_id ?? currentUser.department_id ?? null
          }
          onCreate={onCreateRoom}
          onClose={() => setShowCreate(false)}
        />
      )}

      {showSearch && (
        <SearchModal
          onSearch={onSearch}
          onJump={onJumpToRoom}
          onClose={() => setShowSearch(false)}
        />
      )}

      {showNewDm && (
        <NewDmModal
          users={directory}
          selfName={currentUser.name}
          onStart={onCreateDm}
          onClose={() => setShowNewDm(false)}
        />
      )}

      {showSettings && (
        <SettingsModal
          theme={theme}
          onToggleTheme={onToggleTheme}
          preferences={preferences}
          onSetPreferences={onSetPreferences}
          onClose={() => setShowSettings(false)}
        />
      )}
    </aside>
  );
};
