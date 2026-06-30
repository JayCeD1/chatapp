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

export interface ChatRoom {
  id: number;
  name: string;
  description: string;
  department_id?: number;
  department_name?: string;
  is_private?: boolean;
  user_count?: number;
}

export interface Message {
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
}

export type ViewState = "login" | "workspace";
export type ConnectionMode = "client" | "server";
