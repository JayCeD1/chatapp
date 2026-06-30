import { describe, it, expect, beforeEach } from "vitest";
import { loadProfile, saveProfile } from "./session";

beforeEach(() => {
  localStorage.clear();
});

describe("session profile", () => {
  it("round-trips a valid profile", () => {
    saveProfile({
      username: "Ada",
      email: "ada@x.com",
      departmentId: 3,
      mode: "server",
      serverIp: "10.0.0.2:3625",
    });
    expect(loadProfile()).toEqual({
      username: "Ada",
      email: "ada@x.com",
      departmentId: 3,
      mode: "server",
      serverIp: "10.0.0.2:3625",
    });
  });

  it("returns an empty object when nothing is stored", () => {
    expect(loadProfile()).toEqual({});
  });

  it("never persists a password even if one sneaks into the object", () => {
    saveProfile({
      username: "Ada",
      email: "ada@x.com",
      departmentId: 1,
      mode: "client",
      serverIp: "x",
      // @ts-expect-error — guard against accidental secret persistence
      password: "topsecret",
    });
    expect(localStorage.getItem("nutler.profile")).not.toContain("topsecret");
  });

  it("drops wrong-typed fields from a corrupt profile", () => {
    localStorage.setItem(
      "nutler.profile",
      JSON.stringify({
        username: 42,
        departmentId: "3",
        mode: "bogus",
        serverIp: 99,
        email: "ok@x.com",
      }),
    );
    expect(loadProfile()).toEqual({
      username: undefined,
      email: "ok@x.com",
      departmentId: undefined,
      mode: undefined,
      serverIp: undefined,
    });
  });

  it("returns an empty object for unparseable JSON", () => {
    localStorage.setItem("nutler.profile", "{not json");
    expect(loadProfile()).toEqual({});
  });
});
