import "./App.css";
import { useChatConnection } from "./hooks/useChatConnection";
import { LoginView } from "./components/LoginView";
import { Workspace } from "./components/Workspace";

const Chat = () => {
  const c = useChatConnection();

  if (c.view === "login" || !c.currentUser) {
    return (
      <div className="min-h-dvh w-full flex items-center justify-center bg-[var(--bg)] relative overflow-hidden p-4">
        {/* Subtle accent glow — calm, not the old busy gradient. */}
        <div className="pointer-events-none absolute -top-1/3 left-1/2 -translate-x-1/2 w-[600px] h-[600px] rounded-full bg-[var(--accent)]/10 blur-[140px]" />
        <LoginView
          departments={c.departments}
          mode={c.mode}
          setMode={c.setMode}
          serverIp={c.serverIp}
          setServerIp={c.setServerIp}
          onLogin={c.login}
        />
      </div>
    );
  }

  return (
    <Workspace
      departments={c.departments}
      chatRooms={c.chatRooms}
      currentRoom={c.currentRoom}
      currentUser={c.currentUser}
      messages={c.messages}
      loadingMessages={c.loadingMessages}
      onlineUsers={c.onlineUsers}
      connectionStatus={c.connectionStatus}
      error={c.error}
      onSelectRoom={c.joinRoom}
      onSendMessage={c.sendMessage}
      onLeaveRoom={c.leaveRoom}
      onLogout={c.logout}
      onDismissError={c.dismissError}
    />
  );
};

export default Chat;
