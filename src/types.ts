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
