import { useState, useEffect, useCallback } from "react";
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

export const useChatConnection = () => {
  const [view, setView] = useState<ViewState>("login");
  const [mode, setMode] = useState<ConnectionMode>("client");
  const [serverIp, setServerIp] = useState("127.0.0.1:3625");
  
  const [currentUser, setCurrentUser] = useState<User | null>(null);
  const [currentRoom, setCurrentRoom] = useState<ChatRoom | null>(null);
  
  const [departments, setDepartments] = useState<Department[]>([]);
  const [chatRooms, setChatRooms] = useState<ChatRoom[]>([]);
  const [messages, setMessages] = useState<Message[]>([]);

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
      })) as Message[];
      // Ensure date format is consistent if needed, but backend should send correct structure
      setMessages(msgs);
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
          const m = JSON.parse(e.payload) as any; // incoming payload structure might differ slightly
           
          // The backend sends `created_at` as a timestamp number (seconds) often, based on previous code
          // We need to normalize it to our Message type
          const newMessage: Message = {
            room_id: m.room_id || currentRoom.id, // Fallback if missing
            room: m.room,
            user_id: m.user_id || 0,
            username: m.username,
            message: m.message,
            message_type: m.message_type,
            is_emoji: m.is_emoji || false,
            created_at: new Date(Number(m.created_at) * 1000).toISOString(),
          };

          if (m.room === currentRoom.name) {
            setMessages((prev) => {
              // Deduplicate
              const isDuplicate = prev.some(
                (msg) =>
                  msg.message === newMessage.message &&
                  msg.username === newMessage.username &&
                  Math.abs(new Date(msg.created_at).getTime() - new Date(newMessage.created_at).getTime()) < 1000
              );
              if (isDuplicate) return prev;
              return [...prev, newMessage];
            });
          }
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
  const login = async (username: string, email: string, departmentId: number) => {
    try {
      const user = (await invoke("upsert_user", {
        name: username,
        email,
        departmentId,
      })) as User;

      setCurrentUser(user);
      localStorage.setItem("nutler.userId", String(user.id));

      if (mode === "server") {
        await invoke("server_listen_as_participant", {
          username,
          userId: user.id,
          port: 3625,
          room: user.department_name,
          roomId: user.department_id,
        });
      } else {
        await invoke("client_connect_to_server", {
          host: serverIp,
          username,
          userId: user.id,
          room: user.department_name,
          roomId: user.department_id,
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
          oldRoom: currentUser.department_name, // Note: Simplification, might need tracking previous room
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
       await invoke("leave_room", { userId: currentUser.id, roomId: currentRoom.id });
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
      const command = mode === "server" ? "send_as_server_participant" : "send_as_client";
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
    if (currentUser && currentRoom) {
      await leaveRoom();
    }
    // Additional cleanup if needed
    // Disconnects aren't strictly exposed in the original code, mostly relying on window close or reload
    // but we can clear state
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
