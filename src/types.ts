export interface Department {
  id: number;
  name: string;
}

export interface User {
  id: number;
  name: string;
  email: string;
  department_id: number;
  department_name: string;
}

// A connectable user from the host directory, for invite + DM pickers.
export interface DirectoryUser {
  id: number;
  name: string;
  is_online: boolean;
}

// A Nutler host found on the LAN via UDP discovery. Advisory display hints — the room
// password (Noise handshake) still gates the actual connection.
export interface ServerInfo {
  address: string;
  port: number;
  name: string;
  user_count: number;
}

export interface ChatRoom {
  id: number;
  name: string;
  description: string;
  department_id?: number;
  department_name?: string;
  is_private?: boolean;
  is_dm?: boolean;
  // For DMs, the label derived from the other members (the stored `name` is a synthetic key).
  display_name?: string;
  user_count?: number;
}

// Metadata reference to an attachment on a message (docs/architecture/attachments.md §1).
// Content metadata only — the blob bytes are fetched separately by attachment `id`.
export interface AttachmentRef {
  id: string; // client-generated UUID for this message-attachment instance
  sha256: string; // hex content address of the blob
  name: string; // original filename (display data only)
  mime: string;
  size: number;
  width?: number; // images only
  height?: number;
}

export interface Message {
  version?: number; // wire envelope version (see docs/architecture ADR-0004)
  id?: number; // DB row id (history)
  message_id?: string; // stable UUID from the backend, used for dedup + React keys
  room_id: number;
  room: string;
  user_id: number;
  username: string;
  message: string;
  message_type?: string;
  is_emoji?: boolean;
  created_at: string; // normalized ISO-8601 UTC string
  edited_at?: string | null;
  deleted_at?: string | null;
  attachments?: AttachmentRef[]; // content sub-object; absent on attachment-less messages
  features?: string[]; // host capability flags, carried only on the Identity frame
}

export interface Reaction {
  emoji: string;
  count: number;
  me: boolean;
}

export interface ReactionAggregate {
  message_id: string;
  emoji: string;
  count: number;
  me: boolean;
}

export interface SearchResult {
  message_id?: string;
  room_id: number;
  room_name: string;
  username: string;
  message: string;
  created_at: string;
}

export type ViewState = "login" | "workspace";
export type ConnectionMode = "client" | "server";
