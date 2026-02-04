
import { useChatConnection } from "./hooks/useChatConnection";
import { LoginView } from "./components/LoginView";
import { RoomList } from "./components/RoomList";
import { ChatInterface } from "./components/ChatInterface";
import { Loader2 } from "lucide-react";

// Add some global animation styles inline or verify they exist in Tailwind config.
// Since we used 'animate-fade-in', 'animate-slide-up' etc, we might need to add them to index.css
// For now, standard transitions are handled by class names.

const Chat = () => {
  const {
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
  } = useChatConnection();

  // Background wrapper
  return (
    <div className="min-h-screen w-full bg-[#0f0c29] bg-gradient-to-br from-[#0f0c29] via-[#302b63] to-[#24243e] flex items-center justify-center relative overflow-hidden font-sans">
      {/* Abstract Background Shapes */}
      <div className="absolute top-[-10%] left-[-10%] w-[40%] h-[40%] bg-violet-600/30 rounded-full blur-[120px] animate-pulse" />
      <div className="absolute bottom-[-10%] right-[-10%] w-[40%] h-[40%] bg-fuchsia-600/30 rounded-full blur-[120px] animate-pulse delay-1000" />
      
      {view === "login" && (
        <LoginView
          departments={departments}
          mode={mode}
          setMode={setMode}
          serverIp={serverIp}
          setServerIp={setServerIp}
          onLogin={login}
        />
      )}

      {view === "rooms" && currentUser && (
        <RoomList
          rooms={chatRooms}
          onJoin={joinRoom}
          username={currentUser.name}
        />
      )}

      {view === "chat" && currentRoom && currentUser && (
        <ChatInterface
          currentRoom={currentRoom}
          currentUser={currentUser}
          messages={messages}
          onSendMessage={sendMessage}
          onLeave={leaveRoom}
          onLogout={logout}
        />
      )}

      {/* Fallback/Loading state if mismatched */}
      {view !== "login" && !currentUser && (
        <div className="flex flex-col items-center text-white/50 gap-2">
            <Loader2 className="animate-spin w-8 h-8" />
            <p>Loading session...</p>
        </div>
      )}
    </div>
  );
};

export default Chat;
