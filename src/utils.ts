import { Message } from "./types";

// Up-to-two-letter initials for an avatar.
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

// Deterministic, readable avatar background derived from the name. Returns an HSL
// string with controlled saturation/lightness so text stays legible on top.
export function avatarColor(name: string): string {
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = name.charCodeAt(i) + ((hash << 5) - hash);
    hash |= 0;
  }
  const hue = Math.abs(hash) % 360;
  return `hsl(${hue} 45% 42%)`;
}

export function formatTime(iso: string): string {
  try {
    return new Date(iso).toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return "";
  }
}

// "Today" / "Yesterday" / a full date, for date separators between message days.
export function formatDateSeparator(iso: string): string {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return "";
  const today = new Date();
  const startOf = (x: Date) =>
    new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const dayMs = 86_400_000;
  const diff = (startOf(today) - startOf(d)) / dayMs;
  if (diff === 0) return "Today";
  if (diff === 1) return "Yesterday";
  return d.toLocaleDateString([], {
    weekday: "long",
    month: "short",
    day: "numeric",
  });
}

export function sameDay(a: string, b: string): boolean {
  const da = new Date(a);
  const db = new Date(b);
  return (
    da.getFullYear() === db.getFullYear() &&
    da.getMonth() === db.getMonth() &&
    da.getDate() === db.getDate()
  );
}

// Whether `msg` should be visually grouped under the previous message (same author,
// same message type, within 5 minutes) — so we don't repeat the avatar/name every line.
export function shouldGroup(prev: Message | undefined, msg: Message): boolean {
  if (!prev) return false;
  if (prev.username !== msg.username) return false;
  if ((prev.message_type || "Chat") !== (msg.message_type || "Chat"))
    return false;
  const dt =
    new Date(msg.created_at).getTime() - new Date(prev.created_at).getTime();
  return dt >= 0 && dt < 5 * 60 * 1000;
}

// System (non-chat) events render as centered notices rather than bubbles.
export function isSystem(msg: Message): boolean {
  const t = msg.message_type || "Chat";
  return t !== "Chat";
}
