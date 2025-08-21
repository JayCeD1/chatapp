import { invoke } from "@tauri-apps/api/core";
import React, { useState, useEffect } from "react";
import {
  Send,
  Users,
  Building2,
  MessageCircle,
  LogOut,
  // Settings,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";

const Chat = () => {
  const [currentView, setCurrentView] = useState("login"); // 'login', 'chat', 'rooms'
  const [mode, setMode] = useState("client"); // 'server' or 'client'
  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [departmentId, setDepartmentId] = useState<number | null>(null);
  const [serverIp, setServerIp] = useState("127.0.0.1:3625");
  const [message, setMessage] = useState("");
  const [currentRoom, setCurrentRoom] = useState<ChatRoom | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [departments, setDepartments] = useState<Department[]>([]);
  const [chatRooms, setChatRooms] = useState<ChatRoom[]>([]);
  // const [users, setUsers] = useState<User[]>([]);
  const [showEmojiPicker, setShowEmojiPicker] = useState(false);
  const [currentUser, setCurrentUser] = useState<User | null>(null);

  const emojis = ["😊", "🤔", "😂", "😈", "👍", "👎", "❤️", "🎉", "🔥", "💯"];

  useEffect(() => {
    if (currentView === "login") {
      loadDepartments();
    }
  }, [currentView]);

  useEffect(() => {
    if (currentRoom) {
      loadRoomMessages();
    }
  }, [currentRoom]);

  //On app boot, try auto-login
  useEffect(() => {
    //todo auto login should be able to start either server or client?
    const savedId = localStorage.getItem("nutler.userId");

    if (savedId) {
      invoke("get_user_by_id", { id: Number(savedId) })
        .then((u) => {
          const user = u as User;
          setCurrentUser(user);
          setUsername(user.name);
          setCurrentView("rooms");
          loadChatRooms();
        })
        .catch(() => localStorage.removeItem("nutler.userId"));
    }
  }, []);

  //listen for messages events from the server
  useEffect(() => {
    if (!currentRoom) return;

    let stop: (() => void) | undefined;

    (async () => {
      const unlisten = await listen<string>("message", (e) => {
        console.log("Raw payload:", e.payload);
        console.log("Payload type:", typeof e.payload);
        console.log("Payload length:", e.payload.length);
        console.log("First 100 chars:", e.payload.substring(0, 100));
        console.log(
          "Last 100 chars:",
          e.payload.substring(e.payload.length - 100),
        );

        // Check if it ends properly
        const lastChar = e.payload[e.payload.length - 1];
        console.log(
          "Last character:",
          lastChar,
          "Is closing brace:",
          lastChar === "}",
        );

        try {
          const m = JSON.parse(e.payload) as Message;

          //I think we can include all other message types as well as long us they belong to the same room
          if (m.message_type === "Chat" && m.room === currentRoom?.name) {
            setMessages((prev) => {
              // Check for duplicate messages to prevent duplicates
              const isDuplicate = prev.some(
                (msg) =>
                  msg.message === m.message &&
                  msg.username === m.username &&
                  msg.created_at ===
                    new Date(Number(m.created_at) * 1000).toISOString(),
              );

              if (isDuplicate) {
                return prev; // Don't add if duplicate
              }

              return [
                ...prev,
                {
                  room_id: currentRoom!.id!,
                  room: currentRoom!.name!,
                  user_id: 0,
                  username: m.username,
                  message: m.message,
                  message_type: m.message_type,
                  is_emoji: m.is_emoji,
                  created_at: new Date(
                    Number(m.created_at) * 1000,
                  ).toISOString(),
                },
              ];
            });
          }
        } catch (error) {
          console.error("JSON parse error:", error);
          console.error("Failed payload:", e.payload);
          return; // Skip this message
        }
      });
      stop = () => {
        unlisten();
      };
    })();
    return () => {
      stop?.();
    };
  }, [currentRoom]);

  // Frontend reconnection
  useEffect(() => {
    let stop: (() => void) | undefined;

    (async () => {
      const unlisten = await listen("connection_lost", () => {
        console.log("Connection lost, attempting to reconnect...");
        // Exponential backoff retry
        let retryDelay = 1000;
        const maxRetries = 5;
        let retryCount = 0;

        const attemptReconnect = () => {
          if (retryCount >= maxRetries) {
            console.error("Max reconnection attempts reached");
            return;
          }

          setTimeout(() => {
            invoke("client_connect", {
              host: serverIp,
              username: currentUser?.name || "",
              user_id: currentUser?.id || 0,
              room: currentRoom?.name || "Company Wide",
              room_id: currentRoom?.id || 1,
            })
              .then(() => {
                console.log("Reconnected successfully");
                retryCount = 0;
              })
              .catch((e) => {
                console.error("Reconnect failed:", e);
                retryCount++;
                retryDelay *= 2; // Exponential backoff
                attemptReconnect();
              });
          }, retryDelay);
        };

        attemptReconnect();
      });
      stop = () => {
        unlisten();
      };
    })();

    return () => {
      stop?.();
    };
  }, [serverIp, currentUser, currentRoom]);

  const loadDepartments = async () => {
    try {
      const deps = await invoke("get_departments");
      setDepartments(deps as Department[]);
    } catch (error) {
      console.error("Error loading departments:", error);
    }
  };

  const loadChatRooms = async () => {
    try {
      const rooms = await invoke("get_chat_rooms");
      setChatRooms(rooms as ChatRoom[]);
    } catch (error) {
      console.error("Error loading chat rooms:", error);
    }
  };

  const loadRoomMessages = async () => {
    if (!currentRoom?.id) return;

    try {
      const msgs = await invoke("get_room_messages", {
        roomId: currentRoom.id,
        limit: 50,
      });
      setMessages(msgs as Message[]);
    } catch (error) {
      console.error("Error loading messages:", error);
    }
  };

  const handleJoin = async () => {
    if (username.trim() && email.trim() && departmentId) {
      try {
        // Create or Update user in the database
        const user = (await invoke("upsert_user", {
          name: username,
          email: email,
          departmentId: departmentId,
        })) as User;

        //set current user
        setCurrentUser(user);
        localStorage.setItem("nutler.userId", String(user.id));

        if (mode === "server") {
          await invoke("server_listen", {
            username,
            user_id: user.id,
            port: 3625,
          });
          const addr = (await invoke("get_server_info")) as string;
          const host = addr.replace("0.0.0.0", "127.0.0.1");
          console.log("Server listening on:", host);

          //todo update to match renewed client_connect method this has to be revisited esp room for join company chat button
          await invoke("client_connect", {
            host,
            username,
            user_id: user.id,
            room: user.department_name,
            room_id: user.department_id,
          });
        } else {
          await invoke("client_connect", {
            host: serverIp,
            username,
            user_id: user.id,
            room: user.department_name,
            room_id: user.department_id,
          });
        }

        // Load chat rooms and users
        await loadChatRooms();
        setCurrentView("rooms");
      } catch (error) {
        console.error("Joined failed:", error);
      }
    }
  };

  const handleJoinRoom = async (room: ChatRoom) => {
    if (!currentUser?.id || !room.id) return;

    try {
      await invoke("join_room", { userId: currentUser.id, roomId: room.id });
      setCurrentRoom(room);
      setCurrentView("chat");
    } catch (error) {
      console.error("Error joining room:", error);
    }
  };

  //todo handle leave room as well both (front + back)
  //todo seems sockets send are not persisted in the db!! URGENT!!!
  const handleSendMessage = async () => {
    if (message.trim() && currentRoom?.id && currentUser?.id) {
      try {
        // Send the message via socket
        await invoke("send", {
          message: message,
          user_id: currentUser.id,
          room: currentRoom.name,
          room_id: currentRoom.id,
          is_emoji: false,
        });

        // REMOVE THIS PART - let the listener handle adding messages
        // The message will be added via the listener when it comes back from server
        // This prevents duplicate messages

        // Add to local state
        // const newMessage: Message = {
        //   room_id: currentRoom.id,
        //   room: currentRoom.name,
        //   user_id: currentUser.id,
        //   username: currentUser.name,
        //   message: message,
        //   message_type: "Chat",
        //   is_emoji: false,
        //   created_at: new Date().toISOString(),
        // };

        // setMessages((prev) => [...prev, newMessage]);
        setMessage(""); // Only clear the input text
      } catch (error) {
        console.error("Error sending message:", error);
      }
    }
  };

  const handleEmojiSelect = (emoji: string) => {
    setMessage(message + emoji);
    setShowEmojiPicker(false);
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      handleSendMessage();
    }
  };

  const formatTime = (timestamp: string) => {
    return new Date(timestamp).toLocaleTimeString("en-US", {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
    });
  };

  if (currentView === "login") {
    return (
      <div className="min-h-screen bg-gradient-to-br from-blue-400 to-purple-600 flex items-center justify-center p-4">
        <div className="bg-white rounded-2xl shadow-xl w-full max-w-md p-8 space-y-6">
          {/* Header */}
          <div className="text-center">
            <h1 className="text-3xl font-bold text-gray-800 mb-2">
              Company Chat
            </h1>
            <p className="text-gray-600">Connect with your team</p>
          </div>

          {/* Server/Client Toggle */}
          <div className="bg-gray-100 rounded-full p-1 flex">
            <button
              onClick={() => setMode("server")}
              className={`flex-1 py-3 px-6 rounded-full cursor-pointer font-medium transition-all duration-200 ${
                mode === "server"
                  ? "bg-purple-500 text-white shadow-md"
                  : "text-gray-600 hover:text-gray-800"
              }`}
            >
              Host Server
            </button>
            <button
              onClick={() => setMode("client")}
              className={`flex-1 py-3 px-6 rounded-full cursor-pointer font-medium transition-all duration-200 ${
                mode === "client"
                  ? "bg-purple-500 text-white shadow-md"
                  : "text-gray-600 hover:text-gray-800"
              }`}
            >
              Join Server
            </button>
          </div>

          {/* Server IP Input */}
          <div
            className={`transition-all duration-300 ease-in-out ${
              mode === "client"
                ? "opacity-100 max-h-20"
                : "opacity-0 max-h-0 overflow-hidden"
            }`}
          >
            <input
              type="text"
              value={serverIp}
              onChange={(e) => setServerIp(e.target.value)}
              placeholder="Server IP:Port (e.g., 192.168.1.100:3625)"
              className="w-full px-4 py-3 bg-gray-50 border border-gray-200 rounded-xl focus:outline-none focus:ring-2 focus:ring-purple-500 focus:border-transparent text-gray-700 placeholder-gray-400"
            />
          </div>

          {/* Username Input */}
          <div>
            <input
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="Enter your name"
              className="w-full px-4 py-3 bg-gray-50 border border-gray-200 rounded-xl focus:outline-none focus:ring-2 focus:ring-purple-500 focus:border-transparent text-gray-700 placeholder-gray-400"
            />
          </div>

          {/* Email Input */}
          <div>
            <input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="Enter your email"
              className="w-full px-4 py-3 bg-gray-50 border border-gray-200 rounded-xl focus:outline-none focus:ring-2 focus:ring-purple-500 focus:border-transparent text-gray-700 placeholder-gray-400"
            />
          </div>

          {/* Department Selection */}
          <div>
            <select
              value={departmentId || ""}
              onChange={(e) => setDepartmentId(Number(e.target.value) || null)}
              className="w-full px-4 py-3 bg-gray-50 border border-gray-200 rounded-xl focus:outline-none focus:ring-2 focus:ring-purple-500 focus:border-transparent text-gray-700"
            >
              <option value="">Select your department</option>
              {departments.map((dept) => (
                <option key={dept.id} value={dept.id}>
                  {dept.name}
                </option>
              ))}
            </select>
          </div>

          {/* Join Button */}
          <button
            onClick={handleJoin}
            disabled={!username.trim() || !email.trim() || !departmentId}
            className="w-full bg-purple-500 hover:bg-purple-600 disabled:bg-gray-300 disabled:cursor-not-allowed text-white font-semibold py-4 px-6 rounded-xl transition-colors duration-200 shadow-md hover:shadow-lg"
          >
            Join Company Chat
          </button>

          {/* Status indicator */}
          <div className="text-center">
            <p className="text-sm text-gray-500">
              Mode:{" "}
              <span className="font-semibold text-purple-600 capitalize">
                {mode}
              </span>
              {mode === "client" && serverIp && (
                <span className="block mt-1">
                  Connecting to: <span className="font-mono">{serverIp}</span>
                </span>
              )}
            </p>
          </div>
        </div>
      </div>
    );
  }

  if (currentView === "rooms") {
    return (
      <div className="min-h-screen bg-gray-100 flex">
        {/* Sidebar */}
        <div className="w-80 bg-white shadow-lg">
          <div className="p-6 border-b border-gray-200">
            <h2 className="text-xl font-semibold text-gray-800">Chat Rooms</h2>
            <p className="text-sm text-gray-600 mt-1">Welcome, {username}!</p>
          </div>

          <div className="p-4 space-y-2">
            {chatRooms.map((room) => (
              <button
                key={room.id}
                onClick={() => handleJoinRoom(room)}
                className="w-full text-left p-4 rounded-lg hover:bg-gray-50 border border-gray-200 transition-colors"
              >
                <div className="flex items-center justify-between">
                  <div>
                    <h3 className="font-medium text-gray-800">{room.name}</h3>
                    <p className="text-sm text-gray-600">{room.description}</p>
                    {room.department_name && (
                      <span className="inline-block bg-purple-100 text-purple-800 text-xs px-2 py-1 rounded-full mt-1">
                        {room.department_name}
                      </span>
                    )}
                  </div>
                  <div className="text-right">
                    <div className="flex items-center text-sm text-gray-500">
                      <Users size={16} className="mr-1" />
                      {room.user_count || 0}
                    </div>
                  </div>
                </div>
              </button>
            ))}
          </div>
        </div>

        {/* Main Content */}
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center">
            <MessageCircle size={64} className="text-gray-400 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-gray-600 mb-2">
              Select a Chat Room
            </h3>
            <p className="text-gray-500">
              Choose a room to start chatting with your team
            </p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-100 flex flex-col">
      {/* Chat Header */}
      <div className="bg-white shadow-sm border-b border-gray-200 px-6 py-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center space-x-4">
            <button
              onClick={() => setCurrentView("rooms")}
              className="text-gray-500 hover:text-gray-700 p-2 rounded-lg hover:bg-gray-100"
            >
              <Building2 size={20} />
            </button>
            <div>
              <h2 className="text-lg font-semibold text-gray-800">
                {currentRoom?.name}
              </h2>
              <p className="text-sm text-gray-600">
                {currentRoom?.description}
              </p>
            </div>
          </div>
          <div className="flex items-center space-x-2">
            <span className="text-sm text-gray-500">
              {currentRoom?.user_count || 0} online
            </span>
            <button
              onClick={() => setCurrentView("login")}
              className="text-gray-500 hover:text-gray-700 p-2 rounded-lg hover:bg-gray-100"
            >
              <LogOut size={20} />
            </button>
          </div>
        </div>
      </div>

      {/* Messages Area */}
      <div className="flex-1 overflow-y-auto p-6 space-y-4">
        {messages.map((msg, index) => (
          <div key={index} className="w-full">
            <div
              className={`flex ${msg.username === username ? "justify-end" : "justify-start"}`}
            >
              <div
                className={`max-w-xs lg:max-w-md px-4 py-3 rounded-lg ${
                  msg.username === username
                    ? "bg-purple-500 text-white"
                    : "bg-white border border-gray-200 text-gray-800"
                }`}
              >
                <div className="flex items-center space-x-2 mb-1">
                  <span className="font-medium text-sm">{msg.username}</span>
                  <span
                    className={`text-xs ${
                      msg.username === username
                        ? "text-purple-200"
                        : "text-gray-500"
                    }`}
                  >
                    {formatTime(msg.created_at)}
                  </span>
                </div>
                <p className="text-sm">{msg.message}</p>
              </div>
            </div>
          </div>
        ))}
      </div>

      {/* Emoji Picker */}
      {showEmojiPicker && (
        <div className="bg-white border-t border-gray-200 p-4">
          <div className="flex flex-wrap gap-2 justify-center">
            {emojis.map((emoji, index) => (
              <button
                key={index}
                onClick={() => handleEmojiSelect(emoji)}
                className="text-2xl p-2 hover:bg-gray-100 rounded-lg cursor-pointer transition-colors"
              >
                {emoji}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Input Area */}
      <div className="bg-white border-t border-gray-200 p-4">
        {/* Emoji Bar */}
        <div className="flex justify-center space-x-2 mb-3">
          {emojis.map((emoji, index) => (
            <button
              key={index}
              onClick={() => handleEmojiSelect(emoji)}
              className="text-xl p-1 hover:bg-gray-100 rounded-lg cursor-pointer transition-colors"
            >
              {emoji}
            </button>
          ))}
        </div>

        {/* Message Input */}
        <div className="flex space-x-3">
          <input
            type="text"
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            onKeyDown={handleKeyPress}
            placeholder="Type your message..."
            className="flex-1 px-4 py-3 bg-gray-50 border border-gray-200 rounded-xl focus:outline-none focus:ring-2 focus:ring-purple-500 focus:border-transparent text-gray-700 placeholder-gray-400"
          />
          <button
            onClick={handleSendMessage}
            disabled={!message.trim()}
            className="bg-purple-500 hover:bg-purple-600 disabled:bg-gray-300 disabled:cursor-not-allowed text-white p-3 rounded-xl transition-colors duration-200 shadow-md hover:shadow-lg"
          >
            <Send size={20} />
          </button>
        </div>
      </div>
    </div>
  );
};

export default Chat;
