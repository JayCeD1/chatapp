import { describe, it, expect, beforeEach } from "vitest";
import {
  loadPreferences,
  savePreferences,
  DEFAULT_PREFERENCES,
  Preferences,
} from "./preferences";

beforeEach(() => {
  localStorage.clear();
});

describe("preferences", () => {
  it("round-trips valid preferences", () => {
    const p: Preferences = { notifications: "mentions", sendOnEnter: false };
    savePreferences(p);
    expect(loadPreferences()).toEqual(p);
  });

  it("returns defaults when nothing is stored", () => {
    expect(loadPreferences()).toEqual(DEFAULT_PREFERENCES);
  });

  it("falls back to defaults for wrong-typed or unknown values", () => {
    localStorage.setItem(
      "nutler.preferences",
      JSON.stringify({ notifications: "loud", sendOnEnter: "yes" }),
    );
    expect(loadPreferences()).toEqual(DEFAULT_PREFERENCES);
  });

  it("returns defaults for unparseable JSON", () => {
    localStorage.setItem("nutler.preferences", "{not json");
    expect(loadPreferences()).toEqual(DEFAULT_PREFERENCES);
  });

  it("only persists known fields (no stray/secret leakage)", () => {
    savePreferences({
      notifications: "off",
      sendOnEnter: true,
      // @ts-expect-error — a stray field must not be written to storage
      password: "s3cret",
    });
    const raw = localStorage.getItem("nutler.preferences")!;
    expect(raw).not.toContain("s3cret");
    expect(raw).not.toContain("password");
    expect(loadPreferences()).toEqual({
      notifications: "off",
      sendOnEnter: true,
    });
  });
});
