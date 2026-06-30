import React, { useState } from "react";
import { Hash, LogOut, Users, Sun, Moon, Plus } from "lucide-react";
import { ChatRoom, Department, User } from "../types";
import { ConnectionStatus } from "../hooks/useChatConnection";
import { Theme } from "../hooks/useTheme";
import { initials, avatarColor } from "../utils";
import { CreateChannelModal } from "./CreateChannelModal";

interface SidebarProps {
  departments: Department[];
  chatRooms: ChatRoom[];
  currentRoom: ChatRoom | null;
  currentUser: User;
  connectionStatus: ConnectionStatus;
  onSelectRoom: (room: ChatRoom) => void;
  onCreateRoom: (
    name: string,
    description: string,
    departmentId: number | null,
    isPrivate: boolean,
  ) => Promise<void>;
  onLogout: () => void;
  theme: Theme;
  onToggleTheme: () => void;
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
  connectionStatus,
  onSelectRoom,
  onCreateRoom,
  onLogout,
  theme,
  onToggleTheme,
}) => {
  const [showCreate, setShowCreate] = useState(false);

  // Group rooms by department; keep any unmatched rooms under "Other".
  const groups = departments
    .map((dep) => ({
      name: dep.name,
      rooms: chatRooms.filter((r) => r.department_name === dep.name),
    }))
    .filter((g) => g.rooms.length > 0);

  const matched = new Set(groups.flatMap((g) => g.rooms.map((r) => r.id)));
  const orphans = chatRooms.filter((r) => !matched.has(r.id));
  if (orphans.length) groups.push({ name: "Other", rooms: orphans });

  const status = statusMeta[connectionStatus];

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
              {group.rooms.map((room) => {
                const active = currentRoom?.id === room.id;
                return (
                  <li key={room.id}>
                    <button
                      onClick={() => onSelectRoom(room)}
                      aria-current={active ? "true" : undefined}
                      className={`group w-full flex items-center gap-2 px-2.5 py-1.5 rounded-md text-sm transition-colors ${
                        active
                          ? "bg-[var(--surface-3)] text-[var(--text)]"
                          : "text-[var(--text-dim)] hover:bg-[var(--surface-2)] hover:text-[var(--text)]"
                      }`}
                    >
                      <Hash
                        className={`w-4 h-4 shrink-0 ${
                          active
                            ? "text-[var(--accent-strong)]"
                            : "text-[var(--text-faint)]"
                        }`}
                      />
                      <span className="truncate flex-1 text-left">
                        {room.name}
                      </span>
                      {(room.user_count ?? 0) > 0 && (
                        <span className="flex items-center gap-1 text-[11px] text-[var(--text-faint)]">
                          <Users className="w-3 h-3" />
                          {room.user_count}
                        </span>
                      )}
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
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
    </aside>
  );
};
