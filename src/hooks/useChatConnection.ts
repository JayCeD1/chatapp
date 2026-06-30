import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ChatRoom,
  ConnectionMode,
  Department,
  Message,
  User,
  ViewState,
} from "../types";

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
  const [onlineUsers, setOnlineUsers] = useState<string[]>([]);
  const [connectionStatus, setConnectionStatus] =
    useState<ConnectionStatus>("connected");
  const [error, setError] = useState<string | null>(null);
  const [loadingMessages, setLoadingMessages] = useState(false);

  // Refs so the once-registered listeners read the latest values without re-subscribing.
  const passwordRef = useRef("");
  const currentRoomRef = useRef<ChatRoom | null>(null);
  useEffect(() => {
    currentRoomRef.current = currentRoom;
  }, [currentRoom]);

  // Active room's messages, derived from the store.
  const messages = currentRoom ? messagesByRoom[currentRoom.name] || [] : [];

  // Route an incoming message into the per-room store (deduped) and update presence.
  const ingestMessage = useCallback((m: any) => {
    const nm = normalizeMessage(m);
    if (!nm.room) return;

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

    // Derive live-ish presence from lifecycle system messages.
    const t = nm.message_type;
    if (t === "Connect" || t === "RoomJoin") {
      setOnlineUsers((prev) =>
        prev.includes(nm.username) ? prev : [...prev, nm.username],
      );
    } else if (t === "Disconnect" || t === "RoomLeave") {
      setOnlineUsers((prev) => prev.filter((u) => u !== nm.username));
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
    setLoadingMessages(true);
    try {
      const msgs = (await invoke("get_room_messages", {
        roomId: room.id,
        limit: 50,
      })) as any[];
      const normalized = msgs.map((m) => normalizeMessage(m, room.id));
      setMessagesByRoom((prev) => ({ ...prev, [room.name]: normalized }));
    } catch (err) {
      console.error("Error loading messages:", err);
    } finally {
      setLoadingMessages(false);
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
      setOnlineUsers([user.name]);
      localStorage.setItem("nutler.userId", String(user.id));

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
      loadRoomMessages(room);
    } catch (err) {
      console.error("Join room failed:", err);
      setError(`Couldn't open #${room.name}: ${err}`);
    }
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

  const logout = async () => {
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
    setOnlineUsers([]);
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
    onlineUsers,
    connectionStatus,
    error,
    loadingMessages,
    currentUser,
    currentRoom,
    setMode,
    setServerIp,
    login,
    joinRoom,
    leaveRoom,
    sendMessage,
    logout,
    dismissError,
  };
};
