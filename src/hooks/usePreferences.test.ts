import { describe, it, expect, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { usePreferences } from "./usePreferences";

beforeEach(() => localStorage.clear());

describe("usePreferences", () => {
  it("loads defaults, merges + persists changes, and keeps the ref live", () => {
    const { result } = renderHook(() => usePreferences());
    expect(result.current.preferences).toEqual({
      notifications: "all",
      sendOnEnter: true,
    });

    act(() => result.current.setPreferences({ notifications: "off" }));

    expect(result.current.preferences.notifications).toBe("off");
    // The ref reflects the live value (what the message ingest reads).
    expect(result.current.preferencesRef.current.notifications).toBe("off");
    // Persisted to localStorage.
    expect(
      JSON.parse(localStorage.getItem("nutler.preferences")!).notifications,
    ).toBe("off");
  });
});
