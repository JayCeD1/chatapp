import React, { useState, useEffect, useRef } from "react";
import { Send, LogOut, ArrowLeft, Smile } from "lucide-react";
import { ChatRoom, Message, User } from "../types";

interface ChatInterfaceProps {
  currentRoom: ChatRoom;
  currentUser: User;
  messages: Message[];
  onSendMessage: (text: string, isEmoji?: boolean) => void;
  onLeave: () => void;
  onLogout: () => void;
}

const EMOJIS = ["😊", "🤔", "😂", "🚀", "👍", "👎", "❤️", "🎉", "🔥", "💯", "👀", "🙌"];

export const ChatInterface: React.FC<ChatInterfaceProps> = ({
  currentRoom,
  currentUser,
  messages,
  onSendMessage,
  onLeave,
  onLogout,
}) => {
  const [inputText, setInputText] = useState("");
  const [showEmoji, setShowEmoji] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  const handleSend = () => {
    if (inputText.trim()) {
      onSendMessage(inputText);
      setInputText("");
      setShowEmoji(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const formatTime = (isoString: string) => {
    try {
      return new Date(isoString).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    } catch (e) {
      return "";
    }
  };

  return (
    <div className="flex flex-col w-full h-screen max-w-6xl mx-auto md:p-6 animate-fade-in z-10">
      {/* Header */}
      <div className="bg-white/10 backdrop-blur-xl border-b border-white/10 md:rounded-t-2xl p-4 flex items-center justify-between shadow-sm">
        <div className="flex items-center gap-3">
          <button
            onClick={onLeave}
            className="p-2 hover:bg-white/10 rounded-full text-white/70 hover:text-white transition-colors"
            title="Leave Room"
          >
            <ArrowLeft className="w-5 h-5" />
          </button>
          <div>
            <h2 className="text-white font-bold text-lg leading-tight">{currentRoom.name}</h2>
            <p className="text-white/50 text-xs">{currentRoom.department_name || "General"}</p>
          </div>
        </div>
        
        <div className="flex items-center gap-2">
            <div className="px-3 py-1 bg-white/5 rounded-full text-xs text-emerald-400 font-medium border border-white/5 hidden md:block">
             Online
            </div>
            <button
                onClick={onLogout}
                className="p-2 hover:bg-red-500/20 text-white/70 hover:text-red-400 rounded-lg transition-colors"
                title="Log Out"
            >
                <LogOut className="w-5 h-5" />
            </button>
        </div>
      </div>

      {/* Messages Area */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4 bg-black/20 backdrop-blur-sm scrollbar-thin scrollbar-thumb-white/10 scrollbar-track-transparent">
        {messages.map((msg, idx) => {
          const isMe = msg.username === currentUser.name; // OR use user_id check if available and reliable


          return (
            <div
              key={idx}
              className={`flex ${isMe ? "justify-end" : "justify-start"} group animate-slide-up`}
            >
              <div
                className={`max-w-[80%] md:max-w-[60%] flex flex-col ${
                  isMe ? "items-end" : "items-start"
                }`}
              >
                {!isMe && (
                  <span className="text-xs text-white/50 ml-1 mb-1">{msg.username}</span>
                )}
                
                <div
                  className={`px-4 py-2.5 rounded-2xl shadow-sm text-sm break-words leading-relaxed ${
                    isMe
                      ? "bg-gradient-to-br from-violet-600 to-fuchsia-600 text-white rounded-br-none"
                      : "bg-white/10 border border-white/5 text-white/90 rounded-bl-none hover:bg-white/15 transition-colors"
                  }`}
                >
                  {msg.message}
                </div>
                
                <span className={`text-[10px] text-white/30 mt-1 ${isMe ? "mr-1" : "ml-1"}`}>
                  {formatTime(msg.created_at)}
                </span>
              </div>
            </div>
          );
        })}
        <div ref={messagesEndRef} />
      </div>

      {/* Input Area */}
      <div className="bg-white/10 backdrop-blur-xl border-t border-white/10 md:rounded-b-2xl p-4">
        <div className="relative flex items-end gap-2">
            {/* Emoji Picker Popover */}
            {showEmoji && (
                <div className="absolute bottom-16 left-0 bg-gray-900 border border-white/10 p-3 rounded-xl shadow-2xl grid grid-cols-4 gap-2 z-50 animate-scale-in">
                    {EMOJIS.map(emoji => (
                        <button
                            key={emoji}
                            onClick={() => {
                                setInputText(prev => prev + emoji);
                                // setShowEmoji(false); // Keep open for multi-select?
                            }}
                            className="text-2xl hover:bg-white/10 p-2 rounded-lg transition-colors"
                        >
                            {emoji}
                        </button>
                    ))}
                </div>
            )}

            <button
                onClick={() => setShowEmoji(!showEmoji)}
                className={`p-3 rounded-xl transition-colors ${showEmoji ? 'bg-white/20 text-white' : 'hover:bg-white/10 text-white/60 hover:text-white'}`}
            >
                <Smile className="w-6 h-6" />
            </button>

            <div className="flex-1 bg-white/5 border border-white/10 rounded-xl focus-within:bg-white/10 focus-within:ring-1 focus-within:ring-white/20 transition-all flex items-center">
                <input
                    type="text"
                    value={inputText}
                    onChange={(e) => setInputText(e.target.value)}
                    onKeyDown={handleKeyDown}
                    placeholder="Type a message..."
                    className="w-full bg-transparent border-none text-white placeholder-white/40 px-4 py-3 focus:outline-none"
                />
            </div>

            <button
                onClick={handleSend}
                disabled={!inputText.trim()}
                className="p-3 bg-gradient-to-r from-violet-500 to-fuchsia-500 text-white rounded-xl shadow-lg hover:shadow-violet-500/25 disabled:opacity-50 disabled:cursor-not-allowed transform hover:scale-105 active:scale-95 transition-all"
            >
                <Send className="w-5 h-5" />
            </button>
        </div>
      </div>
    </div>
  );
};
