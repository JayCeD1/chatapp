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

// Short "Mon 3, 10:42 AM" for search results (handles DB "YYYY-MM-DD HH:MM:SS" UTC).
export function formatSearchTime(s: string): string {
  const iso = s.includes("T") ? s : s.replace(" ", "T") + "Z";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return "";
  return d.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
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

// Whether `text` @-mentions the given user (case-insensitive).
export function mentionsUser(text: string, name: string): boolean {
  if (!name) return false;
  return text.toLowerCase().includes("@" + name.toLowerCase());
}

// Split text into runs, flagging @mention tokens for styled rendering.
export function parseMentions(
  text: string,
): { text: string; mention: boolean }[] {
  const parts: { text: string; mention: boolean }[] = [];
  const re = /@[\w.-]+/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last)
      parts.push({ text: text.slice(last, m.index), mention: false });
    parts.push({ text: m[0], mention: true });
    last = m.index + m[0].length;
  }
  if (last < text.length)
    parts.push({ text: text.slice(last), mention: false });
  return parts;
}

// Backend command errors: our typed AppError serializes to { code, message } (see
// src-tauri/src/error.rs); older commands still return a plain string. These normalize both.
export function errText(e: unknown): string {
  if (e && typeof e === "object" && "message" in e)
    return String((e as { message: unknown }).message);
  return String(e);
}

export function errCode(e: unknown): string | undefined {
  if (e && typeof e === "object" && "code" in e)
    return String((e as { code: unknown }).code);
  return undefined;
}
