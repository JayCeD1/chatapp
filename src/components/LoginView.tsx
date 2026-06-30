import React, { useState, useEffect } from "react";
import {
  User,
  Server,
  Hash,
  Lock,
  Mail,
  AlertCircle,
  Wifi,
} from "lucide-react";
import { Department, ConnectionMode, ServerInfo } from "../types";
import { loadProfile } from "../session";

interface LoginViewProps {
  departments: Department[];
  mode: ConnectionMode;
  setMode: (mode: ConnectionMode) => void;
  serverIp: string;
  setServerIp: (ip: string) => void;
  onDiscover?: () => Promise<ServerInfo[]>;
  onLogin: (
    username: string,
    email: string,
    departmentId: number,
    password: string,
  ) => Promise<void>;
}

const inputClass =
  "w-full bg-[var(--surface-2)] border border-[var(--border)] rounded-xl py-3 pl-10 pr-4 text-[var(--text)] placeholder-[var(--text-faint)] focus:outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent-soft)] transition-colors";

export const LoginView: React.FC<LoginViewProps> = ({
  departments,
  mode,
  setMode,
  serverIp,
  setServerIp,
  onDiscover,
  onLogin,
}) => {
  // Pre-fill from the remembered profile (never the password).
  const [username, setUsername] = useState(() => loadProfile().username ?? "");
  const [email, setEmail] = useState(() => loadProfile().email ?? "");
  const [departmentId, setDepartmentId] = useState<number | null>(
    () => loadProfile().departmentId ?? null,
  );
  const [password, setPassword] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // LAN discovery (client mode): results + state for the "Find hosts" affordance.
  const [discovered, setDiscovered] = useState<ServerInfo[]>([]);
  const [discovering, setDiscovering] = useState(false);
  const [discoverNote, setDiscoverNote] = useState<string | null>(null);

  const handleDiscover = async () => {
    if (!onDiscover || discovering) return;
    setDiscovering(true);
    setDiscoverNote(null);
    try {
      const found = await onDiscover();
      setDiscovered(found);
      setDiscoverNote(
        found.length === 0 ? "No hosts found on your network." : null,
      );
    } catch {
      setDiscoverNote("Discovery failed.");
    } finally {
      setDiscovering(false);
    }
  };
  // If the form is pre-filled, drop the user straight on the password field.
  const [returning] = useState(() => !!loadProfile().username);

  // Drop stale discovery results/notes when switching connection mode (the component stays
  // mounted across the client/server toggle, so this state would otherwise persist).
  useEffect(() => {
    setDiscovered([]);
    setDiscoverNote(null);
  }, [mode]);

  // A restored department id may no longer exist (department list changed, or a
  // corrupt profile). Once the live list loads, drop a stale id so the form can't
  // submit a blank-but-truthy selection into a wrong/None room.
  useEffect(() => {
    if (
      departmentId != null &&
      departments.length > 0 &&
      !departments.some((d) => d.id === departmentId)
    ) {
      setDepartmentId(null);
    }
  }, [departments, departmentId]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!username || !email || !departmentId || !password) return;
    setLoading(true);
    setError(null);
    try {
      await onLogin(username, email, departmentId, password);
    } catch (err) {
      setError(
        String(err).toLowerCase().includes("handshake")
          ? "Couldn't connect — check the server address and room password."
          : "Couldn't connect. Please check your details and try again.",
      );
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="w-full max-w-md p-8 rounded-2xl bg-[var(--surface)] border border-[var(--border)] shadow-2xl animate-fade-in relative z-10">
      <div className="text-center mb-7">
        <div className="inline-flex items-center justify-center w-12 h-12 bg-gradient-to-br from-[var(--accent)] to-[var(--accent-strong)] rounded-xl shadow-lg mb-4">
          <Hash className="text-white w-6 h-6" />
        </div>
        <h1 className="text-2xl font-bold text-[var(--text)] tracking-tight">
          Welcome to Nutler
        </h1>
        <p className="text-[var(--text-dim)] mt-1.5 text-sm">
          {mode === "server"
            ? "Host a room for your team"
            : "Connect to your team workspace"}
        </p>
      </div>

      <div
        className="bg-[var(--surface-2)] p-1 rounded-xl flex mb-5"
        role="tablist"
        aria-label="Connection mode"
      >
        <button
          type="button"
          role="tab"
          aria-selected={mode === "client"}
          onClick={() => setMode("client")}
          className={`flex-1 py-2 rounded-lg text-sm font-semibold transition-colors ${
            mode === "client"
              ? "bg-[var(--surface-3)] text-[var(--text)]"
              : "text-[var(--text-dim)] hover:text-[var(--text)]"
          }`}
        >
          Join Server
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={mode === "server"}
          onClick={() => setMode("server")}
          className={`flex-1 py-2 rounded-lg text-sm font-semibold transition-colors ${
            mode === "server"
              ? "bg-[var(--surface-3)] text-[var(--text)]"
              : "text-[var(--text-dim)] hover:text-[var(--text)]"
          }`}
        >
          Host Server
        </button>
      </div>

      <form className="space-y-3.5" onSubmit={handleSubmit}>
        {mode === "client" && (
          <div className="space-y-2">
            <Field icon={<Server className="w-4 h-4" />} label="Server address">
              <input
                type="text"
                value={serverIp}
                onChange={(e) => setServerIp(e.target.value)}
                placeholder="Server IP (e.g. 127.0.0.1:3625)"
                className={inputClass}
              />
            </Field>
            {onDiscover && (
              <div>
                <button
                  type="button"
                  onClick={handleDiscover}
                  disabled={discovering}
                  aria-busy={discovering}
                  className="flex items-center gap-1.5 text-xs font-medium text-[var(--accent-strong)] hover:underline disabled:opacity-60 disabled:no-underline"
                >
                  {discovering ? (
                    <span className="w-3.5 h-3.5 border-2 border-[var(--accent-strong)]/30 border-t-[var(--accent-strong)] rounded-full animate-spin" />
                  ) : (
                    <Wifi className="w-3.5 h-3.5" />
                  )}
                  {discovering ? "Searching…" : "Find hosts on your network"}
                </button>
                {/* Live region so screen readers announce results / the note as they appear. */}
                <div role="status" aria-live="polite">
                  {discovered.length > 0 && (
                    <ul className="mt-1.5 border border-[var(--border)] rounded-lg divide-y divide-[var(--border)] overflow-hidden">
                      {discovered.map((s) => (
                        <li key={`${s.address}:${s.port}`}>
                          <button
                            type="button"
                            onClick={() => {
                              setServerIp(`${s.address}:${s.port}`);
                              setDiscovered([]);
                              setDiscoverNote(null);
                            }}
                            className="w-full text-left px-3 py-2 hover:bg-[var(--surface-2)] transition-colors"
                          >
                            <div className="text-sm text-[var(--text)] truncate">
                              {s.name}
                            </div>
                            <div className="text-[11px] text-[var(--text-faint)]">
                              {s.address}:{s.port} · {s.user_count} online
                            </div>
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                  {discoverNote && (
                    <p className="mt-1 text-[11px] text-[var(--text-faint)]">
                      {discoverNote}
                    </p>
                  )}
                </div>
              </div>
            )}
          </div>
        )}

        <Field icon={<Lock className="w-4 h-4" />} label="Room password">
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={
              mode === "server" ? "Set a room password" : "Room password"
            }
            autoComplete={
              mode === "server" ? "new-password" : "current-password"
            }
            autoFocus={returning}
            className={inputClass}
          />
        </Field>

        <Field icon={<User className="w-4 h-4" />} label="Username">
          <input
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            placeholder="Username"
            autoFocus={!returning}
            className={inputClass}
          />
        </Field>

        <Field icon={<Mail className="w-4 h-4" />} label="Email address">
          <input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="Email address"
            className={inputClass}
          />
        </Field>

        <div>
          <label htmlFor="department" className="sr-only">
            Department
          </label>
          <select
            id="department"
            value={departmentId || ""}
            onChange={(e) => setDepartmentId(Number(e.target.value))}
            className="w-full bg-[var(--surface-2)] border border-[var(--border)] rounded-xl py-3 px-4 text-[var(--text)] focus:outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent-soft)] transition-colors appearance-none cursor-pointer"
          >
            <option value="" disabled>
              Select department
            </option>
            {departments.map((dep) => (
              <option key={dep.id} value={dep.id}>
                {dep.name}
              </option>
            ))}
          </select>
        </div>

        {error && (
          <div
            role="alert"
            className="flex items-start gap-2 text-sm text-[var(--danger)] bg-[var(--danger)]/10 border border-[var(--danger)]/30 rounded-lg px-3 py-2"
          >
            <AlertCircle className="w-4 h-4 mt-0.5 shrink-0" />
            <span>{error}</span>
          </div>
        )}

        <button
          type="submit"
          disabled={
            loading || !username || !email || !departmentId || !password
          }
          className="w-full bg-[var(--accent)] hover:bg-[var(--accent-strong)] text-white font-semibold py-3 rounded-xl shadow-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed mt-1"
        >
          {loading ? (
            <span className="flex items-center justify-center gap-2">
              <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
              Connecting…
            </span>
          ) : mode === "server" ? (
            "Start hosting"
          ) : (
            "Enter workspace"
          )}
        </button>
      </form>
    </div>
  );
};

const Field: React.FC<{
  icon: React.ReactNode;
  label: string;
  children: React.ReactNode;
}> = ({ icon, label, children }) => (
  <div className="relative">
    <label className="sr-only">{label}</label>
    <span className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-faint)]">
      {icon}
    </span>
    {children}
  </div>
);
