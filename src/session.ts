import { ConnectionMode } from "./types";

// Remembers the last *non-secret* login fields so the form pre-fills on relaunch.
// The room password is the encryption PSK and is deliberately NEVER persisted.
const KEY = "nutler.profile";

export interface SessionProfile {
  username: string;
  email: string;
  departmentId: number | null;
  mode: ConnectionMode;
  serverIp: string;
}

export const loadProfile = (): Partial<SessionProfile> => {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return {};
    const p = JSON.parse(raw) as Record<string, unknown>;
    // Validate each field by type — a corrupt or hand-edited profile must never
    // push wrong-typed values into form state or down to the backend.
    return {
      username: typeof p.username === "string" ? p.username : undefined,
      email: typeof p.email === "string" ? p.email : undefined,
      departmentId:
        typeof p.departmentId === "number" ? p.departmentId : undefined,
      mode:
        p.mode === "server" || p.mode === "client"
          ? (p.mode as ConnectionMode)
          : undefined,
      serverIp: typeof p.serverIp === "string" ? p.serverIp : undefined,
    };
  } catch {
    return {};
  }
};

export const saveProfile = (p: SessionProfile): void => {
  // Persist ONLY the known non-secret fields by name, so a stray property on the
  // caller's object (e.g. a password) can never leak into storage.
  const safe: SessionProfile = {
    username: p.username,
    email: p.email,
    departmentId: p.departmentId,
    mode: p.mode,
    serverIp: p.serverIp,
  };
  try {
    localStorage.setItem(KEY, JSON.stringify(safe));
  } catch {
    /* storage may be unavailable (private mode); non-fatal */
  }
};
