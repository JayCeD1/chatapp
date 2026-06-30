import React, { useState } from "react";
import { User, Server, Globe, Lock } from "lucide-react";
import { Department, ConnectionMode } from "../types";

interface LoginViewProps {
  departments: Department[];
  mode: ConnectionMode;
  setMode: (mode: ConnectionMode) => void;
  serverIp: string;
  setServerIp: (ip: string) => void;
  onLogin: (
    username: string,
    email: string,
    departmentId: number,
    password: string,
  ) => Promise<void>;
}

export const LoginView: React.FC<LoginViewProps> = ({
  departments,
  mode,
  setMode,
  serverIp,
  setServerIp,
  onLogin,
}) => {
  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [departmentId, setDepartmentId] = useState<number | null>(null);
  const [password, setPassword] = useState("");
  const [loading, setLoading] = useState(false);

  const handleSubmit = async () => {
    if (!username || !email || !departmentId || !password) return;
    setLoading(true);
    try {
      await onLogin(username, email, departmentId, password);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="w-full max-w-md p-8 rounded-3xl bg-white/10 backdrop-blur-xl border border-white/20 shadow-2xl animate-fade-in relative z-10">
      <div className="text-center mb-8">
        <div className="inline-flex items-center justify-center p-4 bg-gradient-to-br from-violet-500 to-fuchsia-500 rounded-2xl shadow-lg mb-4">
          <Globe className="text-white w-8 h-8" />
        </div>
        <h1 className="text-3xl font-bold text-white tracking-tight">
          Welcome Back
        </h1>
        <p className="text-white/60 mt-2">Connect with your team workspace</p>
      </div>

      <div className="bg-black/20 p-1 rounded-xl flex mb-6 backdrop-blur-md">
        <button
          onClick={() => setMode("client")}
          className={`flex-1 py-2.5 rounded-lg text-sm font-semibold transition-all duration-300 ${
            mode === "client"
              ? "bg-white text-violet-900 shadow-md transform scale-105"
              : "text-white/70 hover:text-white"
          }`}
        >
          Join Server
        </button>
        <button
          onClick={() => setMode("server")}
          className={`flex-1 py-2.5 rounded-lg text-sm font-semibold transition-all duration-300 ${
            mode === "server"
              ? "bg-white text-violet-900 shadow-md transform scale-105"
              : "text-white/70 hover:text-white"
          }`}
        >
          Host Server
        </button>
      </div>

      <div className="space-y-4">
        <div
          className={`transition-all duration-300 overflow-hidden ${
            mode === "client" ? "max-h-20 opacity-100" : "max-h-0 opacity-0"
          }`}
        >
          <div className="relative group">
            <Server className="absolute left-3 top-3.5 w-5 h-5 text-white/50 group-focus-within:text-white transition-colors" />
            <input
              type="text"
              value={serverIp}
              onChange={(e) => setServerIp(e.target.value)}
              placeholder="Server IP (e.g. 127.0.0.1:3625)"
              className="w-full bg-white/5 border border-white/10 rounded-xl py-3 pl-10 pr-4 text-white placeholder-white/30 focus:outline-none focus:ring-2 focus:ring-violet-400/50 focus:bg-white/10 transition-all"
            />
          </div>
        </div>

        <div className="relative group">
          <Lock className="absolute left-3 top-3.5 w-5 h-5 text-white/50 group-focus-within:text-white transition-colors" />
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={
              mode === "server" ? "Set a room password" : "Room password"
            }
            className="w-full bg-white/5 border border-white/10 rounded-xl py-3 pl-10 pr-4 text-white placeholder-white/30 focus:outline-none focus:ring-2 focus:ring-violet-400/50 focus:bg-white/10 transition-all"
          />
        </div>

        <div className="relative group">
          <User className="absolute left-3 top-3.5 w-5 h-5 text-white/50 group-focus-within:text-white transition-colors" />
          <input
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            placeholder="Username"
            className="w-full bg-white/5 border border-white/10 rounded-xl py-3 pl-10 pr-4 text-white placeholder-white/30 focus:outline-none focus:ring-2 focus:ring-violet-400/50 focus:bg-white/10 transition-all"
          />
        </div>

        <div className="relative group">
          <input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="Email Address"
            className="w-full bg-white/5 border border-white/10 rounded-xl py-3 px-4 text-white placeholder-white/30 focus:outline-none focus:ring-2 focus:ring-violet-400/50 focus:bg-white/10 transition-all"
          />
        </div>

        <div className="relative">
          <select
            value={departmentId || ""}
            onChange={(e) => setDepartmentId(Number(e.target.value))}
            className="w-full bg-white/5 border border-white/10 rounded-xl py-3 px-4 text-white focus:outline-none focus:ring-2 focus:ring-violet-400/50 focus:bg-white/10 transition-all appearance-none cursor-pointer"
          >
            <option value="" className="bg-gray-800 text-gray-300">
              Select Department
            </option>
            {departments.map((dep) => (
              <option
                key={dep.id}
                value={dep.id}
                className="bg-gray-800 text-white"
              >
                {dep.name}
              </option>
            ))}
          </select>
          <div className="absolute right-4 top-3.5 pointer-events-none">
            <svg
              className="w-5 h-5 text-white/50"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth="2"
                d="M19 9l-7 7-7-7"
              ></path>
            </svg>
          </div>
        </div>

        <button
          onClick={handleSubmit}
          disabled={
            loading || !username || !email || !departmentId || !password
          }
          className="w-full bg-gradient-to-r from-violet-500 to-fuchsia-500 hover:from-violet-400 hover:to-fuchsia-400 text-white font-bold py-3.5 rounded-xl shadow-lg hover:shadow-violet-500/25 transform hover:-translate-y-0.5 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed disabled:transform-none mt-4"
        >
          {loading ? (
            <span className="flex items-center justify-center gap-2">
              <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
              Connecting...
            </span>
          ) : (
            "Enter Workspace"
          )}
        </button>
      </div>
    </div>
  );
};
