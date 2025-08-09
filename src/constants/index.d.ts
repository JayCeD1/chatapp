interface User {
  id?: number;
  name: string;
  email: string;
  department_id?: number;
  department_name?: string;
  is_online: boolean;
  last_seen?: string;
}

interface Department {
  id?: number;
  name: string;
  description?: string;
}

interface ChatRoom {
  id?: number;
  name: string;
  description?: string;
  department_id?: number;
  department_name?: string;
  is_private: boolean;
  user_count?: number;
}

interface Message {
  id?: number;
  room_id: number;
  room: string;
  user_id: number;
  username: string;
  message: string;
  message_type: string;
  is_emoji: boolean;
  created_at: string;
}

interface InsertResult {
  rows_affected: number;
  last_insert_id: number;
}
