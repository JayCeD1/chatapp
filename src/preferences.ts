// Persisted, non-secret app preferences (localStorage). Mirrors src/session.ts: a single
// JSON key, hand-rolled per-field validation, and try/catch around all storage access so
// private-mode / unavailable storage is non-fatal. Unlike session's Partial return, this
// always resolves to a full, valid Preferences with defaults filled in.

export type NotificationMode = "all" | "mentions" | "off";

export interface Preferences {
  // Desktop notification level:
  //   all      — notify on any message while the window is unfocused, and always on @mention
  //   mentions — notify only when @mentioned
  //   off      — never notify
  notifications: NotificationMode;
  // true: Enter sends (Shift+Enter does not). false: Enter does nothing (avoids accidental
  // sends). Cmd/Ctrl+Enter always sends in either mode.
  sendOnEnter: boolean;
}

const KEY = "nutler.preferences";

export const DEFAULT_PREFERENCES: Preferences = {
  notifications: "all",
  sendOnEnter: true,
};

function isNotificationMode(v: unknown): v is NotificationMode {
  return v === "all" || v === "mentions" || v === "off";
}

export function loadPreferences(): Preferences {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { ...DEFAULT_PREFERENCES };
    const p = JSON.parse(raw) as Record<string, unknown>;
    return {
      notifications: isNotificationMode(p.notifications)
        ? p.notifications
        : DEFAULT_PREFERENCES.notifications,
      sendOnEnter:
        typeof p.sendOnEnter === "boolean"
          ? p.sendOnEnter
          : DEFAULT_PREFERENCES.sendOnEnter,
    };
  } catch {
    return { ...DEFAULT_PREFERENCES };
  }
}

export function savePreferences(p: Preferences): void {
  try {
    // Construct an explicit allowlisted object so a stray/secret field can never leak.
    const safe: Preferences = {
      notifications: p.notifications,
      sendOnEnter: p.sendOnEnter,
    };
    localStorage.setItem(KEY, JSON.stringify(safe));
  } catch {
    /* storage unavailable (private mode); non-fatal */
  }
}
