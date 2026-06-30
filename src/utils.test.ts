import { describe, it, expect } from "vitest";
import {
  initials,
  avatarColor,
  sameDay,
  shouldGroup,
  isSystem,
  mentionsUser,
  parseMentions,
} from "./utils";
import { Message } from "./types";

const msg = (over: Partial<Message>): Message => ({
  id: 1,
  message_id: "m",
  room_id: 1,
  room: "general",
  user_id: 1,
  username: "Alice",
  message: "hi",
  message_type: "Chat",
  is_emoji: false,
  created_at: "2026-06-30T10:00:00.000Z",
  edited_at: null,
  deleted_at: null,
  ...over,
});

describe("initials", () => {
  it("takes the first two letters of a single name", () => {
    expect(initials("alice")).toBe("AL");
  });
  it("takes first + last initials for multi-word names", () => {
    expect(initials("Ada Lovelace")).toBe("AL");
    expect(initials("  mary  jane  watson ")).toBe("MW");
  });
  it("falls back to ? for empty input", () => {
    expect(initials("   ")).toBe("?");
  });
});

describe("avatarColor", () => {
  it("is deterministic for the same name", () => {
    expect(avatarColor("Bob")).toBe(avatarColor("Bob"));
  });
  it("returns a well-formed HSL string", () => {
    expect(avatarColor("Bob")).toMatch(/^hsl\(\d{1,3} 45% 42%\)$/);
  });
});

describe("sameDay", () => {
  // Local-time strings (no trailing Z) so the assertions don't depend on the runner's
  // timezone — sameDay compares local calendar dates.
  it("is true within the same calendar day", () => {
    expect(sameDay("2026-06-30T08:00:00", "2026-06-30T20:00:00")).toBe(true);
  });
  it("is false across days", () => {
    expect(sameDay("2026-06-29T08:00:00", "2026-06-30T08:00:00")).toBe(false);
  });
});

describe("shouldGroup", () => {
  it("returns false with no previous message", () => {
    expect(shouldGroup(undefined, msg({}))).toBe(false);
  });
  it("groups same author + type within 5 minutes", () => {
    const prev = msg({ created_at: "2026-06-30T10:00:00.000Z" });
    const next = msg({ created_at: "2026-06-30T10:04:00.000Z" });
    expect(shouldGroup(prev, next)).toBe(true);
  });
  it("does not group different authors", () => {
    const prev = msg({ username: "Bob" });
    expect(shouldGroup(prev, msg({}))).toBe(false);
  });
  it("does not group across a >5min gap", () => {
    const prev = msg({ created_at: "2026-06-30T10:00:00.000Z" });
    const next = msg({ created_at: "2026-06-30T10:06:00.000Z" });
    expect(shouldGroup(prev, next)).toBe(false);
  });
});

describe("isSystem", () => {
  it("treats Chat as a normal message", () => {
    expect(isSystem(msg({ message_type: "Chat" }))).toBe(false);
  });
  it("treats non-Chat types as system notices", () => {
    expect(isSystem(msg({ message_type: "RoomJoin" }))).toBe(true);
  });
});

describe("mentionsUser", () => {
  it("matches case-insensitively", () => {
    expect(mentionsUser("hey @Alice!", "alice")).toBe(true);
  });
  it("is false when not mentioned", () => {
    expect(mentionsUser("hey bob", "alice")).toBe(false);
  });
  it("is false for an empty name", () => {
    expect(mentionsUser("@", "")).toBe(false);
  });
});

describe("parseMentions", () => {
  it("splits text into plain and mention runs", () => {
    expect(parseMentions("hi @bob and @ada")).toEqual([
      { text: "hi ", mention: false },
      { text: "@bob", mention: true },
      { text: " and ", mention: false },
      { text: "@ada", mention: true },
    ]);
  });
  it("returns a single plain run when there are no mentions", () => {
    expect(parseMentions("just text")).toEqual([
      { text: "just text", mention: false },
    ]);
  });
});
