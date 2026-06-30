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
  const [messages, setMessages] = useState<Message[]>([]);

  // Held in a ref (not state/localStorage) so reconnect can re-derive the Noise key
  // without persisting the room password to disk.
  const passwordRef = useRef("");

  // Load departments on mount
  useEffect(() => {
    loadDepartments();
  }, []);

  const loadDepartments = async () => {
    try {
      const deps = (await invoke("get_departments")) as Department[];
      setDepartments(deps);
    } catch (error) {
      console.error("Error loading departments:", error);
    }
  };

  const loadChatRooms = async () => {
    try {
      const rooms = (await invoke("get_chat_rooms")) as ChatRoom[];
      setChatRooms(rooms);
    } catch (error) {
      console.error("Error loading chat rooms:", error);
    }
  };

  const loadRoomMessages = useCallback(async (roomId: number) => {
    try {
      const msgs = (await invoke("get_room_messages", {
        roomId,
        limit: 50,
      })) as any[];
      // Normalize so history and live messages share one timestamp format.
      setMessages(msgs.map((m) => normalizeMessage(m, roomId)));
    } catch (error) {
      console.error("Error loading messages:", error);
    }
  }, []);

  // Listen for incoming messages
  useEffect(() => {
    if (!currentRoom) return;

    let unlisten: () => void;

    const setupListener = async () => {
      unlisten = await listen<string>("message", (e) => {
        try {
          // Lifecycle events use their own channels; ignore empty/non-chat payloads.
          if (!e.payload) return;
          const m = JSON.parse(e.payload) as any;
          if (!m || m.room !== currentRoom.name) return;

          const newMessage = normalizeMessage(m, currentRoom.id);

          setMessages((prev) => {
            // Primary dedup: the stable backend message_id (covers reconnect echoes
            // and identical messages sent in quick succession).
            if (
              newMessage.message_id &&
              prev.some((p) => p.message_id === newMessage.message_id)
            ) {
              return prev;
            }
            // Fallback for legacy rows that predate message_id.
            const isDuplicate = prev.some(
              (msg) =>
                !msg.message_id &&
                msg.message === newMessage.message &&
                msg.username === newMessage.username &&
                Math.abs(
                  new Date(msg.created_at).getTime() -
                    new Date(newMessage.created_at).getTime(),
                ) < 1000,
            );
            if (isDuplicate) return prev;
            return [...prev, newMessage];
          });
        } catch (error) {
          console.error("Error parsing message:", error);
        }
      });
    };

    setupListener();

    return () => {
      if (unlisten) unlisten();
    };
  }, [currentRoom]);

  // Reconnection logic
  useEffect(() => {
    // Only attempt reconnect if we have a user and are in client mode
    if (mode !== "client" || !currentUser) return;

    let unlisten: () => void;

    const setupReconnect = async () => {
      unlisten = await listen("connection_lost", () => {
        console.log("Connection lost, attempting to reconnect...");
        let retryDelay = 1000;
        let retryCount = 0;
        const maxRetries = 5;

        const attemptReconnect = () => {
          if (retryCount >= maxRetries) return;

          setTimeout(() => {
            invoke("client_connect_to_server", {
              host: serverIp,
              username: currentUser.name,
              userId: currentUser.id,
              room: currentRoom?.department_name || currentUser.department_name, // Fallback
              roomId: currentRoom?.id || currentUser.department_id,
              password: passwordRef.current,
            })
              .then(() => {
                console.log("Reconnected successfully");
                retryCount = 0;
              })
              .catch(() => {
                retryCount++;
                retryDelay *= 2;
                attemptReconnect();
              });
          }, retryDelay);
        };
        attemptReconnect();
      });
    };

    setupReconnect();
    return () => {
      if (unlisten) unlisten();
    };
  }, [mode, currentUser, currentRoom, serverIp]);

  // Actions
  const login = async (
    username: string,
    email: string,
    departmentId: number,
    password: string,
  ) => {
    try {
      const user = (await invoke("upsert_user", {
        name: username,
        email,
        departmentId,
      })) as User;

      passwordRef.current = password;
      setCurrentUser(user);
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
      setView("rooms");
    } catch (error) {
      console.error("Login failed:", error);
      throw error;
    }
  };

  const joinRoom = async (room: ChatRoom) => {
    if (!currentUser) return;

    try {
      if (mode === "server") {
        await invoke("server_participant_join_room", {
          userId: currentUser.id,
          newRoom: room.name,
          newRoomId: room.id,
          // Use the actual previous room so the host removes us from the right bucket;
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
      setView("chat");
      loadRoomMessages(room.id);
    } catch (error) {
      console.error("Join room failed:", error);
    }
  };

  const leaveRoom = async () => {
    if (!currentUser || !currentRoom) return;
    try {
      await invoke("leave_room", {
        userId: currentUser.id,
        roomId: currentRoom.id,
      });
      setCurrentRoom(null);
      setView("rooms");
      setMessages([]);
    } catch (error) {
      console.error("Leave room failed:", error);
    }
  };

  const sendMessage = async (text: string, isEmoji = false) => {
    if (!currentUser || !currentRoom) return;
    try {
      const command =
        mode === "server" ? "send_as_server_participant" : "send_as_client";
      await invoke(command, {
        message: text,
        user_id: currentUser.id,
        is_emoji: isEmoji,
      });
    } catch (error) {
      console.error("Send message failed:", error);
    }
  };

  const logout = async () => {
    // Tear down the live TCP connection (client) or stop hosting (server) so the
    // socket and bound port are actually released — otherwise re-login fails with
    // "address already in use" and the host keeps ghost clients.
    try {
      if (mode === "server") {
        await invoke("server_participant_disconnect");
      } else {
        await invoke("client_disconnect");
      }
    } catch (error) {
      console.error("Disconnect failed:", error);
    }

    // Mark the room inactive in the DB if we were in one.
    if (currentUser && currentRoom) {
      try {
        await invoke("leave_room", {
          userId: currentUser.id,
          roomId: currentRoom.id,
        });
      } catch (error) {
        console.error("Leave room on logout failed:", error);
      }
    }

    setCurrentUser(null);
    setCurrentRoom(null);
    setMessages([]);
    setView("login");
    localStorage.removeItem("nutler.userId");
  };

  return {
    view,
    mode,
    serverIp,
    departments,
    chatRooms,
    messages,
    currentUser,
    currentRoom,
    setMode,
    setServerIp,
    login,
    joinRoom,
    leaveRoom,
    sendMessage,
    logout,
  };
};
