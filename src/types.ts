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
  department_name?: string;
  user_count?: number;
}

export interface Message {
  room_id: number;
  room: string;
  user_id: number;
  username: string;
  message: string;
  message_type?: string;
  is_emoji?: boolean;
  created_at: string; // ISO string
}

export type ViewState = "login" | "rooms" | "chat";
export type ConnectionMode = "client" | "server";
