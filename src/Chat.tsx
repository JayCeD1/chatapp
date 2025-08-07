import {invoke} from "@tauri-apps/api/core";
import {useState} from "react";
import { Send } from "lucide-react";

const Chat = () => {
    const [currentView, setCurrentView] = useState('login'); // 'login' or 'chat'
    const [mode, setMode] = useState('client'); // 'server' or 'client'
    const [username, setUsername] = useState('');
    const [serverIp, setServerIp] = useState('127.0.0.1:3625');
    const [message, setMessage] = useState('');
    const [messages, setMessages] = useState([
        { user: 'Jane', text: 'Jane connected', time: '23:21', type: 'system' }
    ]);
    const [showEmojiPicker, setShowEmojiPicker] = useState(false);

    const emojis = ['😊', '🤔', '😂', '😊', '😈', '😈', '😊', '😊'];

    const handleJoin = () => {
        if (username.trim()) {
            setCurrentView('chat');
        }
    };

    const handleSendMessage = () => {
        if (message.trim()) {
            const now = new Date();
            const time = now.toLocaleTimeString('en-US', {
                hour12: false,
                hour: '2-digit',
                minute: '2-digit'
            });

            setMessages([...messages, {
                user: username,
                text: message,
                time: time,
                type: 'message'
            }]);
            setMessage('');
        }
    };

    const handleEmojiSelect = (emoji) => {
        setMessage(message + emoji);
        setShowEmojiPicker(false);
    };

    const handleKeyPress = (e) => {
        if (e.key === 'Enter') {
            handleSendMessage();
        }
    };

    if (currentView === 'login') {
        return (
            <div className="min-h-screen bg-gradient-to-br from-gray-400 to-gray-700 flex items-center justify-center p-4">
                <div className="bg-white rounded-2xl shadow-xl w-full max-w-md p-8 space-y-6">
                    {/* Header */}
                    <div className="text-center">
                        <h1 className="text-2xl font-bold text-gray-800 mb-2">Welcome to Rust Chat!</h1>
                    </div>

                    {/* Server/Client Toggle */}
                    <div className="bg-gray-100 rounded-full p-1 flex">
                        <button
                            onClick={() => setMode('server')}
                            className={`flex-1 py-3 px-6 rounded-full cursor-pointer font-medium transition-all duration-200 ${
                                mode === 'server'
                                    ? 'bg-purple-500 text-white shadow-md'
                                    : 'text-gray-600 hover:text-gray-800'
                            }`}
                        >
                            Server
                        </button>
                        <button
                            onClick={() => setMode('client')}
                            className={`flex-1 py-3 px-6 rounded-full cursor-pointer font-medium transition-all duration-200 ${
                                mode === 'client'
                                    ? 'bg-purple-500 text-white shadow-md'
                                    : 'text-gray-600 hover:text-gray-800'
                            }`}
                        >
                            Client
                        </button>
                    </div>

                    {/* Server IP Input */}
                    <div className={`transition-all duration-300 ease-in-out ${
                        mode === 'client' ? 'opacity-100 max-h-20' : 'opacity-0 max-h-0 overflow-hidden'
                    }`}>
                        <input
                            type="text"
                            value={serverIp}
                            onChange={(e) => setServerIp(e.target.value)}
                            placeholder="Server IP:Port"
                            className="w-full px-4 py-3 bg-gray-50 border border-gray-200 rounded-xl focus:outline-none focus:ring-2 focus:ring-purple-500 focus:border-transparent text-gray-700 placeholder-gray-400"
                        />
                    </div>

                    {/* Username Input */}
                    <div>
                        <input
                            type="text"
                            value={username}
                            onChange={(e) => setUsername(e.target.value)}
                            onKeyPress={(e) => e.key === 'Enter' && handleJoin()}
                            placeholder="Enter a username"
                            className="w-full px-4 py-3 bg-gray-50 border border-gray-200 rounded-xl focus:outline-none focus:ring-2 focus:ring-purple-500 focus:border-transparent text-gray-700 placeholder-gray-400"
                        />
                    </div>

                    {/* Join Button */}
                    <button
                        onClick={handleJoin}
                        className="w-full bg-purple-500 hover:bg-purple-600 cursor-pointer text-white font-semibold py-4 px-6 rounded-xl transition-colors duration-200 shadow-md hover:shadow-lg"
                    >
                        Join
                    </button>

                    {/* Status indicator */}
                    <div className="text-center">
                        <p className="text-sm text-gray-500">
                            Mode: <span className="font-semibold text-purple-600 capitalize">{mode}</span>
                            {mode === 'client' && serverIp && (
                                <span className="block mt-1">Connecting to: <span className="font-mono">{serverIp}</span></span>
                            )}
                        </p>
                    </div>
                </div>
            </div>
        );
    }

    return (
        <div className="min-h-screen bg-gray-100 flex flex-col">
            {/* Chat Header */}
            <div className="bg-white shadow-sm border-b border-gray-200 px-4 py-3">
                <div className="flex items-center justify-between">
                    <h2 className="text-lg font-semibold text-gray-800">{username}</h2>
                    <button
                        onClick={() => setCurrentView('login')}
                        className="text-sm cursor-pointer text-gray-500 hover:text-gray-700 px-3 py-1 rounded-md hover:bg-gray-100"
                    >
                        Leave
                    </button>
                </div>
            </div>

            {/* Messages Area */}
            <div className="flex-1 overflow-y-auto p-4 space-y-3">
                {messages.map((msg, index) => (
                    <div key={index} className="w-full">
                        {msg.type === 'system' ? (
                            <div className="bg-gray-200 rounded-lg p-3 max-w-xs">
                                <p className="text-sm text-gray-700">{msg.text}</p>
                                <p className="text-xs text-gray-500 mt-1">{msg.time}</p>
                            </div>
                        ) : (
                            <div className={`flex ${msg.user === username ? 'justify-end' : 'justify-start'}`}>
                                <div className={`max-w-xs lg:max-w-md px-4 py-2 rounded-lg ${
                                    msg.user === username
                                        ? 'bg-purple-500 text-white'
                                        : 'bg-white border border-gray-200 text-gray-800'
                                }`}>
                                    <p className="text-sm">{msg.text}</p>
                                    <p className={`text-xs mt-1 ${
                                        msg.user === username ? 'text-purple-200' : 'text-gray-500'
                                    }`}>
                                        {msg.time}
                                    </p>
                                </div>
                            </div>
                        )}
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
                        placeholder="Message"
                        className="flex-1 px-4 py-3 bg-gray-50 border border-gray-200 rounded-xl focus:outline-none focus:ring-2 focus:ring-purple-500 focus:border-transparent text-gray-700 placeholder-gray-400"
                    />
                    <button
                        onClick={handleSendMessage}
                        className="bg-purple-500 hover:bg-purple-600 text-white p-3 cursor-pointer  rounded-xl transition-colors duration-200 shadow-md hover:shadow-lg"
                    >
                        <Send size={20} />
                    </button>
                </div>
            </div>
        </div>
    );
}

export default Chat;