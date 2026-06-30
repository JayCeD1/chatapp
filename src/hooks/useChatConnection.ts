import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ChatRoom,
  ConnectionMode,
  Department,
  Message,
  Reaction,
  ReactionAggregate,
  SearchResult,
  User,
  ViewState,
} from "../types";
import { mentionsUser } from "../utils";
import { notify, ensureNotificationPermission } from "../notifications";

export type ConnectionStatus = "connected" | "reconnecting" | "disconnected";

// Normalize a message from either source into one shape with an ISO-8601 UTC timestamp,
// so the UI never has to branch on origin:
//   - live socket: `created_at` is epoch-seconds (number)
//   - DB history:  `created_at` is a UTC "YYYY-MM-DD HH:MM:SS" string
const normalizeMessage = (m: any, fallbackRoomId?: number): Message => {
  const raw = m?.created_at;
  let createdAt: string;
  if (
    typeof raw === "number" ||
    (typeof raw === "string" && /^\d+$/.test(raw))
  ) {
    createdAt = new Date(Number(raw) * 1000).toISOString();
  } else if (typeof raw === "string") {
    // ISO strings pass through; bare "YYYY-MM-DD HH:MM:SS" is UTC — make it explicit.
    const iso = raw.includes("T") ? raw : raw.replace(" ", "T") + "Z";
    const d = new Date(iso);
    createdAt = isNaN(d.getTime()) ? new Date().toISOString() : d.toISOString();
  } else {
    createdAt = new Date().toISOString();
  }

  return {
    id: m?.id,
    message_id: m?.message_id,
    room_id: m?.room_id ?? fallbackRoomId ?? 0,
    room: m?.room,
    user_id: m?.user_id ?? 0,
    username: m?.username,
    message: m?.message,
    message_type: m?.message_type,
    is_emoji: m?.is_emoji ?? false,
    created_at: createdAt,
    edited_at: m?.edited_at ?? null,
    deleted_at: m?.deleted_at ?? null,
  };
};

export const useChatConnection = () => {
  const [view, setView] = useState<ViewState>("login");
  const [mode, setMode] = useState<ConnectionMode>("client");
  const [serverIp, setServerIp] = useState("127.0.0.1:3625");

  const [currentUser, setCurrentUser] = useState<User | null>(null);
  const [currentRoom, setCurrentRoom] = useState<ChatRoom | null>(null);

  const [departments, setDepartments] = useState<Department[]>([]);
  const [chatRooms, setChatRooms] = useState<ChatRoom[]>([]);
  // Per-room message store, keyed by room name (what messages carry on the wire).
  const [messagesByRoom, setMessagesByRoom] = useState<
    Record<string, Message[]>
  >({});
  // Live roster per room (server truth via UserList messages), keyed by room name.
  const [membersByRoom, setMembersByRoom] = useState<Record<string, string[]>>(
    {},
  );
  // Whether older history may still exist per room (false once a short page returns).
  const [hasMoreByRoom, setHasMoreByRoom] = useState<Record<string, boolean>>(
    {},
  );
  // Reactions keyed by target message_id.
  const [reactionsByMessage, setReactionsByMessage] = useState<
    Record<string, Reaction[]>
  >({});
  const [connectionStatus, setConnectionStatus] =
    useState<ConnectionStatus>("connected");
  const [error, setError] = useState<string | null>(null);
  // Per-room loading state — a single global flag would let one room's history
  // arrival clear another room's spinner (or get stuck if its push never comes).
  const [loadingByRoom, setLoadingByRoom] = useState<Record<string, boolean>>(
    {},
  );

  // Refs so the once-registered listeners read the latest values without re-subscribing.
  const passwordRef = useRef("");
  const currentRoomRef = useRef<ChatRoom | null>(null);
  useEffect(() => {
    currentRoomRef.current = currentRoom;
  }, [currentRoom]);
  const messagesByRoomRef = useRef(messagesByRoom);
  useEffect(() => {
    messagesByRoomRef.current = messagesByRoom;
  }, [messagesByRoom]);
  const currentUserRef = useRef<User | null>(null);
  useEffect(() => {
    currentUserRef.current = currentUser;
  }, [currentUser]);

  const PAGE_SIZE = 50;

  // Active room's messages, derived from the store.
  const messages = currentRoom ? messagesByRoom[currentRoom.name] || [] : [];

  // Route an incoming message into the per-room store (deduped); UserList updates the
  // live roster instead of appearing as a chat message.
  const ingestMessage = useCallback((m: any) => {
    const nm = normalizeMessage(m);
    if (!nm.room) return;

    // Host → client scrollback: a JSON batch of {messages, reactions} for a room.
    if (nm.message_type === "HistoryResponse") {
      try {
        const batch = JSON.parse(nm.message) as {
          messages: any[];
          reactions: any[];
        };
        const msgs = (batch.messages || []).map((m) =>
          normalizeMessage({ ...m, room: nm.room }),
        );
        // Merge, not replace. A live Chat/Edit/Delete can land in the gap between
        // the host snapshotting history and this push arriving:
        //  - a brand-new live message at/after the snapshot's newest is kept;
        //  - a live edit/delete to a snapshotted message wins over the stale
        //    snapshot copy (so the gap mutation isn't reverted);
        //  - older live extras are stale and intentionally discarded.
        // Sort by id (the host's authoritative order) and fall back to created_at
        // only for live, id-less rows.
        setMessagesByRoom((prev) => {
          const existing = prev[nm.room] || [];
          const newest = msgs.length ? msgs[msgs.length - 1].created_at : "";
          const byId = new Map<string, Message>();
          for (const m of msgs) if (m.message_id) byId.set(m.message_id, m);
          for (const m of existing) {
            if (!m.message_id) continue;
            const snap = byId.get(m.message_id);
            if (!snap) {
              if (m.created_at >= newest) byId.set(m.message_id, m); // raced-in live msg
            } else {
              const liveAdvanced =
                (m.deleted_at && !snap.deleted_at) ||
                (m.edited_at && (!snap.edited_at || m.edited_at > snap.edited_at));
              if (liveAdvanced) byId.set(m.message_id, { ...m, id: m.id ?? snap.id });
            }
          }
          const merged = [...byId.values()].sort((a, b) => {
            if (a.id != null && b.id != null) return a.id - b.id;
            return a.created_at < b.created_at
              ? -1
              : a.created_at > b.created_at
                ? 1
                : 0;
          });
          return { ...prev, [nm.room]: merged };
        });
        // Recent batch only (full pagination is host-side for now).
        setHasMoreByRoom((prev) => ({ ...prev, [nm.room]: false }));

        const byMsg: Record<string, Reaction[]> = {};
        for (const r of batch.reactions || []) {
          (byMsg[r.message_id] ||= []).push({
            emoji: r.emoji,
            count: r.count,
            me: r.me,
          });
        }
        setReactionsByMessage((prev) => {
          const next = { ...prev };
          for (const m of msgs) {
            if (m.message_id) next[m.message_id] = byMsg[m.message_id] || [];
          }
          return next;
        });
        setLoadingByRoom((p) => ({ ...p, [nm.room]: false }));
      } catch (err) {
        console.error("Bad history payload:", err);
        setLoadingByRoom((p) => ({ ...p, [nm.room]: false }));
      }
      return;
    }

    if (nm.message_type === "UserList") {
      try {
        const names = JSON.parse(nm.message) as string[];
        if (Array.isArray(names)) {
          setMembersByRoom((prev) => ({ ...prev, [nm.room]: names }));
        }
      } catch {
        // ignore malformed roster
      }
      return;
    }

    // Reaction: message_id = target, message = emoji, is_emoji = added(true)/removed.
    if (nm.message_type === "Reaction") {
      const target = nm.message_id;
      const emoji = nm.message;
      const added = !!nm.is_emoji;
      const byMe = nm.user_id === currentUserRef.current?.id;
      if (!target || !emoji) return;
      setReactionsByMessage((prev) => {
        const list = (prev[target] || []).slice();
        const idx = list.findIndex((r) => r.emoji === emoji);
        if (added) {
          if (idx >= 0) {
            list[idx] = {
              ...list[idx],
              count: list[idx].count + 1,
              me: list[idx].me || byMe,
            };
          } else {
            list.push({ emoji, count: 1, me: byMe });
          }
        } else if (idx >= 0) {
          const count = list[idx].count - 1;
          if (count <= 0) list.splice(idx, 1);
          else
            list[idx] = {
              ...list[idx],
              count,
              me: byMe ? false : list[idx].me,
            };
        }
        return { ...prev, [target]: list };
      });
      return;
    }

    // Edit/Delete carry the TARGET message id in message_id; mutate the existing row.
    if (nm.message_type === "Edit" || nm.message_type === "Delete") {
      const deleted = nm.message_type === "Delete";
      setMessagesByRoom((prev) => {
        const list = prev[nm.room];
        if (!list) return prev;
        return {
          ...prev,
          [nm.room]: list.map((m) =>
            m.message_id && m.message_id === nm.message_id
              ? deleted
                ? { ...m, message: "", deleted_at: new Date().toISOString() }
                : {
                    ...m,
                    message: nm.message,
                    edited_at: new Date().toISOString(),
                  }
              : m,
          ),
        };
      });
      return;
    }

    setMessagesByRoom((prev) => {
      const list = prev[nm.room] || [];
      if (nm.message_id && list.some((p) => p.message_id === nm.message_id)) {
        return prev;
      }
      const dup = list.some(
        (p) =>
          !p.message_id &&
          p.message === nm.message &&
          p.username === nm.username &&
          Math.abs(
            new Date(p.created_at).getTime() -
              new Date(nm.created_at).getTime(),
          ) < 1000,
      );
      if (dup) return prev;
      return { ...prev, [nm.room]: [...list, nm] };
    });

    // Desktop notification for chat messages from others, when the window isn't
    // focused or we've been @-mentioned.
    const me = currentUserRef.current;
    if (
      me &&
      (nm.message_type === "Chat" || !nm.message_type) &&
      nm.username !== me.name
    ) {
      const mentioned = mentionsUser(nm.message, me.name);
      if (!document.hasFocus() || mentioned) {
        notify(`#${nm.room}`, `${nm.username}: ${nm.message}`);
      }
    }
  }, []);

  // Load departments on mount.
  useEffect(() => {
    loadDepartments();
  }, []);

  const loadDepartments = async () => {
    try {
      const deps = (await invoke("get_departments")) as Department[];
      setDepartments(deps);
    } catch (err) {
      console.error("Error loading departments:", err);
    }
  };

  const loadChatRooms = async () => {
    try {
      const rooms = (await invoke("get_chat_rooms")) as ChatRoom[];
      setChatRooms(rooms);
    } catch (err) {
      console.error("Error loading chat rooms:", err);
    }
  };

  const loadRoomMessages = useCallback(async (room: ChatRoom) => {
    setLoadingByRoom((prev) => ({ ...prev, [room.name]: true }));
    try {
      const msgs = (await invoke("get_room_messages", {
        roomId: room.id,
        limit: PAGE_SIZE,
      })) as any[];
      const normalized = msgs.map((m) => normalizeMessage(m, room.id));
      setMessagesByRoom((prev) => ({ ...prev, [room.name]: normalized }));
      setHasMoreByRoom((prev) => ({
        ...prev,
        [room.name]: normalized.length >= PAGE_SIZE,
      }));

      // Seed reactions for this room.
      const me = currentUserRef.current?.id ?? 0;
      const reax = (await invoke("get_room_reactions", {
        roomId: room.id,
        userId: me,
      })) as ReactionAggregate[];
      const byMsg: Record<string, Reaction[]> = {};
      for (const r of reax) {
        (byMsg[r.message_id] ||= []).push({
          emoji: r.emoji,
          count: r.count,
          me: r.me,
        });
      }
      // Authoritative, room-scoped replace: set each loaded message's reactions to the
      // aggregate (or [] if none) so emptied reactions can't leave phantom chips. A plain
      // spread merge could never clear a key the aggregate no longer returns.
      setReactionsByMessage((prev) => {
        const next = { ...prev };
        for (const m of normalized) {
          if (m.message_id) next[m.message_id] = byMsg[m.message_id] || [];
        }
        return next;
      });
    } catch (err) {
      console.error("Error loading messages:", err);
    } finally {
      setLoadingByRoom((prev) => ({ ...prev, [room.name]: false }));
    }
  }, []);

  // Load the page of messages immediately older than the oldest one currently held
  // for the active room, and prepend them (deduped).
  const loadOlderMessages = useCallback(async () => {
    const room = currentRoomRef.current;
    if (!room) return;
    const list = messagesByRoomRef.current[room.name] || [];
    const oldest = list.find((m) => m.id != null);
    if (!oldest?.id) return;
    try {
      const older = (await invoke("get_room_messages", {
        roomId: room.id,
        limit: PAGE_SIZE,
        beforeId: oldest.id,
      })) as any[];
      const normalized = older.map((m) => normalizeMessage(m, room.id));
      setHasMoreByRoom((prev) => ({
        ...prev,
        [room.name]: normalized.length >= PAGE_SIZE,
      }));
      if (normalized.length === 0) return;
      setMessagesByRoom((prev) => {
        const existing = prev[room.name] || [];
        const seen = new Set(
          existing.map((m) => m.message_id).filter(Boolean) as string[],
        );
        const fresh = normalized.filter(
          (m) => !m.message_id || !seen.has(m.message_id),
        );
        return { ...prev, [room.name]: [...fresh, ...existing] };
      });
    } catch (err) {
      console.error("Load older failed:", err);
    }
  }, []);

  // Incoming-message listener — registered ONCE; routes every message to the store.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      unlisten = await listen<string>("message", (e) => {
        if (!e.payload) return; // lifecycle events use their own channels
        try {
          const m = JSON.parse(e.payload);
          if (m) ingestMessage(m);
        } catch (err) {
          console.error("Error parsing message:", err);
        }
      });
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [ingestMessage]);

  // Reconnection — registered once per (mode, user, serverIp); reads room from a ref.
  useEffect(() => {
    if (mode !== "client" || !currentUser) return;

    let unlisten: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let retryCount = 0;
    let retryDelay = 1000;
    const maxRetries = 6;

    const attempt = () => {
      if (retryCount >= maxRetries) {
        setConnectionStatus("disconnected");
        return;
      }
      timer = setTimeout(() => {
        const room = currentRoomRef.current;
        invoke("client_connect_to_server", {
          host: serverIp,
          username: currentUser.name,
          userId: currentUser.id,
          room: room?.name || currentUser.department_name,
          roomId: room?.id || currentUser.department_id,
          password: passwordRef.current,
        })
          .then(() => {
            retryCount = 0;
            retryDelay = 1000;
            setConnectionStatus("connected");
          })
          .catch(() => {
            retryCount++;
            retryDelay = Math.min(retryDelay * 2, 15000);
            attempt();
          });
      }, retryDelay);
    };

    (async () => {
      unlisten = await listen("connection_lost", () => {
        setConnectionStatus("reconnecting");
        retryCount = 0;
        retryDelay = 1000;
        attempt();
      });
    })();

    return () => {
      if (unlisten) unlisten();
      if (timer) clearTimeout(timer);
    };
  }, [mode, currentUser, serverIp]);

  // Actions
  const login = async (
    username: string,
    email: string,
    departmentId: number,
    password: string,
  ) => {
    setError(null);
    try {
      const user = (await invoke("upsert_user", {
        name: username,
        email,
        departmentId,
      })) as User;

      passwordRef.current = password;
      setCurrentUser(user);
      localStorage.setItem("nutler.userId", String(user.id));
      ensureNotificationPermission(); // ask once, up front

      // Best-effort DB presence flag (last-seen / online).
      invoke("update_user_online_status", {
        userId: user.id,
        isOnline: true,
      }).catch(() => {});

      if (mode === "server") {
        await invoke("server_listen_as_participant", {
          username,
          userId: user.id,
          port: 3625,
          room: user.department_name,
          roomId: user.department_id,
          password,
        });
      } else {
        await invoke("client_connect_to_server", {
          host: serverIp,
          username,
          userId: user.id,
          room: user.department_name,
          roomId: user.department_id,
          password,
        });
      }

      await loadChatRooms();
      setConnectionStatus("connected");
      setView("workspace");
    } catch (err) {
      console.error("Login failed:", err);
      const msg = String(err);
      setError(
        msg.toLowerCase().includes("handshake")
          ? "Couldn't connect — check the server address and room password."
          : `Couldn't connect: ${msg}`,
      );
      throw err;
    }
  };

  const joinRoom = async (room: ChatRoom) => {
    if (!currentUser) return;
    if (currentRoom?.id === room.id) return; // already open
    try {
      if (mode === "server") {
        await invoke("server_participant_join_room", {
          userId: currentUser.id,
          newRoom: room.name,
          newRoomId: room.id,
          // Real previous room so the host removes us from the right bucket;
          // fall back to the department room only on first join.
          oldRoom: currentRoom?.name || currentUser.department_name,
        });
      } else {
        await invoke("client_join_room", {
          userId: currentUser.id,
          newRoom: room.name,
          newRoomId: room.id,
        });
      }

      await invoke("join_room", { userId: currentUser.id, roomId: room.id });
      setCurrentRoom(room);
      if (mode === "server") {
        // Host owns the data — read it locally.
        loadRoomMessages(room);
      } else {
        // Client's local DB has none of the host's history; show a loading
        // state and wait for the host's HistoryResponse push (see ingestMessage).
        setLoadingByRoom((prev) => ({ ...prev, [room.name]: true }));
        setMessagesByRoom((prev) =>
          prev[room.name] ? prev : { ...prev, [room.name]: [] },
        );
      }
    } catch (err) {
      console.error("Join room failed:", err);
      setError(`Couldn't open #${room.name}: ${err}`);
    }
  };

  // Create a new channel, refresh the list, and open it.
  const createRoom = async (
    name: string,
    description: string,
    departmentId: number | null,
    isPrivate: boolean,
  ) => {
    if (!currentUser) return;
    const room = (await invoke("create_room", {
      name,
      description: description || null,
      departmentId: departmentId ?? null,
      isPrivate,
      createdBy: currentUser.id,
    })) as ChatRoom;
    await loadChatRooms();
    await joinRoom(room);
  };

  const searchMessages = useCallback(
    async (query: string): Promise<SearchResult[]> => {
      try {
        return (await invoke("search_messages", {
          query,
          limit: 50,
        })) as SearchResult[];
      } catch (err) {
        console.error("Search failed:", err);
        return [];
      }
    },
    [],
  );

  // Open a room by id (e.g. from a search result).
  const jumpToRoom = (roomId: number) => {
    const room = chatRooms.find((r) => r.id === roomId);
    if (room) joinRoom(room);
  };

  // Leave the active room (back to "no channel selected"); stays in the workspace.
  const leaveRoom = async () => {
    if (!currentUser || !currentRoom) return;
    const room = currentRoom;
    try {
      if (mode === "server") {
        await invoke("server_leave_room", {
          userId: currentUser.id,
          room: room.name,
          roomId: room.id,
        });
      } else {
        await invoke("client_leave_room", {
          userId: currentUser.id,
          room: room.name,
          roomId: room.id,
        });
      }
      await invoke("leave_room", { userId: currentUser.id, roomId: room.id });
      setCurrentRoom(null);
    } catch (err) {
      console.error("Leave room failed:", err);
    }
  };

  const sendMessage = async (text: string, isEmoji = false) => {
    if (!currentUser || !currentRoom) return;
    try {
      const command =
        mode === "server" ? "send_as_server_participant" : "send_as_client";
      // The backend echoes the sent message back to our UI, so it lands via the
      // listener — no separate optimistic insert needed.
      await invoke(command, {
        message: text,
        user_id: currentUser.id,
        is_emoji: isEmoji,
      });
    } catch (err) {
      console.error("Send message failed:", err);
      setError(`Message not sent: ${err}`);
    }
  };

  const editMessage = async (targetId: string, newText: string) => {
    if (!currentUser || !currentRoom) return;
    const cmd =
      mode === "server" ? "server_edit_message" : "client_edit_message";
    try {
      await invoke(cmd, {
        userId: currentUser.id,
        targetId,
        newText,
        room: currentRoom.name,
        roomId: currentRoom.id,
      });
    } catch (err) {
      setError(`Couldn't edit message: ${err}`);
    }
  };

  const deleteMessage = async (targetId: string) => {
    if (!currentUser || !currentRoom) return;
    const cmd =
      mode === "server" ? "server_delete_message" : "client_delete_message";
    try {
      await invoke(cmd, {
        userId: currentUser.id,
        targetId,
        room: currentRoom.name,
        roomId: currentRoom.id,
      });
    } catch (err) {
      setError(`Couldn't delete message: ${err}`);
    }
  };

  const toggleReaction = async (targetId: string, emoji: string) => {
    if (!currentUser || !currentRoom) return;
    const cmd =
      mode === "server" ? "server_toggle_reaction" : "client_toggle_reaction";
    try {
      await invoke(cmd, {
        userId: currentUser.id,
        targetId,
        emoji,
        room: currentRoom.name,
        roomId: currentRoom.id,
      });
    } catch (err) {
      setError(`Couldn't react: ${err}`);
    }
  };

  const logout = async () => {
    if (currentUser) {
      invoke("update_user_online_status", {
        userId: currentUser.id,
        isOnline: false,
      }).catch(() => {});
    }
    // Tear down the live TCP connection / stop hosting so the socket + port free up.
    try {
      if (mode === "server") {
        await invoke("server_participant_disconnect");
      } else {
        await invoke("client_disconnect");
      }
    } catch (err) {
      console.error("Disconnect failed:", err);
    }

    if (currentUser && currentRoom) {
      try {
        await invoke("leave_room", {
          userId: currentUser.id,
          roomId: currentRoom.id,
        });
      } catch (err) {
        console.error("Leave room on logout failed:", err);
      }
    }

    setCurrentUser(null);
    setCurrentRoom(null);
    setMessagesByRoom({});
    setMembersByRoom({});
    setReactionsByMessage({});
    setConnectionStatus("connected");
    setView("login");
    localStorage.removeItem("nutler.userId");
  };

  const dismissError = () => setError(null);

  return {
    view,
    mode,
    serverIp,
    departments,
    chatRooms,
    messages,
    messagesByRoom,
    membersByRoom,
    connectionStatus,
    error,
    // Derived: only the active room's spinner is surfaced to the UI.
    loadingMessages: currentRoom ? !!loadingByRoom[currentRoom.name] : false,
    hasMore: currentRoom ? (hasMoreByRoom[currentRoom.name] ?? true) : false,
    currentUser,
    currentRoom,
    setMode,
    setServerIp,
    login,
    joinRoom,
    createRoom,
    leaveRoom,
    sendMessage,
    editMessage,
    deleteMessage,
    toggleReaction,
    reactionsByMessage,
    loadOlderMessages,
    searchMessages,
    jumpToRoom,
    logout,
    dismissError,
  };
};
