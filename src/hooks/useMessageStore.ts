import { useState, useEffect, useRef } from "react";
import { ChatRoom, Message, Reaction } from "../types";

export type TypingByRoom = Record<
  string,
  Record<number, { username: string; at: number }>
>;

/// Per-room message data, split out of the connection hook: messages/reactions/roster/unread
/// and their loading + pagination flags, plus the typing-expiry ticker and a ref the
/// once-registered socket listener reads. The connection hook still owns ingest + actions and
/// drives these setters; this hook just owns the state so "message data" is separable/testable.
export function useMessageStore(currentRoom: ChatRoom | null) {
  // Per-room message store, keyed by room name (what messages carry on the wire).
  const [messagesByRoom, setMessagesByRoom] = useState<
    Record<string, Message[]>
  >({});
  // Live roster per room (server truth via UserList messages), keyed by room name.
  const [membersByRoom, setMembersByRoom] = useState<Record<string, string[]>>(
    {},
  );
  // Whether older history may still exist per room (false once a short page returns).
  const [hasMoreByRoom, setHasMoreByRoom] = useState<Record<string, boolean>>(
    {},
  );
  // Reactions keyed by target message_id.
  const [reactionsByMessage, setReactionsByMessage] = useState<
    Record<string, Reaction[]>
  >({});
  // Per-room loading state — a single global flag would let one room's history arrival clear
  // another room's spinner (or get stuck if its push never comes).
  const [loadingByRoom, setLoadingByRoom] = useState<Record<string, boolean>>(
    {},
  );
  // Ephemeral "who is typing" per room: userId → { username, last-seen ms }.
  const [typingByRoom, setTypingByRoom] = useState<TypingByRoom>({});
  // Unread message counts per room id (host computes them; client gets pushes).
  const [unreadByRoom, setUnreadByRoom] = useState<Record<number, number>>({});

  // Ref so the once-registered socket listener reads the latest messages without re-subscribing.
  const messagesByRoomRef = useRef(messagesByRoom);
  useEffect(() => {
    messagesByRoomRef.current = messagesByRoom;
  }, [messagesByRoom]);

  // Expire stale typing entries (>5s) so a dropped "stop" can't pin an indicator on. Returns
  // the same reference when nothing changed, so idle rooms don't re-render.
  useEffect(() => {
    const id = setInterval(() => {
      setTypingByRoom((prev) => {
        const now = Date.now();
        let changed = false;
        const next: typeof prev = {};
        for (const room in prev) {
          const keep: Record<number, { username: string; at: number }> = {};
          for (const uid in prev[room]) {
            const e = prev[room][uid as unknown as number];
            if (now - e.at < 5000) keep[uid as unknown as number] = e;
            else changed = true;
          }
          if (Object.keys(keep).length) next[room] = keep;
          else if (Object.keys(prev[room]).length) changed = true;
        }
        return changed ? next : prev;
      });
    }, 2000);
    return () => clearInterval(id);
  }, []);

  // Active room's messages, derived from the store.
  const messages = currentRoom ? messagesByRoom[currentRoom.name] || [] : [];

  // Clear everything (on logout).
  const reset = () => {
    setMessagesByRoom({});
    setMembersByRoom({});
    setReactionsByMessage({});
    setUnreadByRoom({});
    setHasMoreByRoom({});
    setLoadingByRoom({});
    setTypingByRoom({});
  };

  return {
    messagesByRoom,
    setMessagesByRoom,
    membersByRoom,
    setMembersByRoom,
    hasMoreByRoom,
    setHasMoreByRoom,
    reactionsByMessage,
    setReactionsByMessage,
    loadingByRoom,
    setLoadingByRoom,
    typingByRoom,
    setTypingByRoom,
    unreadByRoom,
    setUnreadByRoom,
    messagesByRoomRef,
    messages,
    reset,
  };
}
