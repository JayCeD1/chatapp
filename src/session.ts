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
    return raw ? (JSON.parse(raw) as Partial<SessionProfile>) : {};
  } catch {
    return {};
  }
};

export const saveProfile = (p: SessionProfile): void => {
  try {
    localStorage.setItem(KEY, JSON.stringify(p));
  } catch {
    /* storage may be unavailable (private mode); non-fatal */
  }
};
