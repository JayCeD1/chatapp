import { describe, it, expect, vi, beforeEach } from "vitest";
import { type ComponentProps } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LoginView } from "./LoginView";
import { Department, ServerInfo } from "../types";

const departments: Department[] = [
  { id: 1, name: "Engineering" },
  { id: 2, name: "Design" },
];

const renderLogin = (
  overrides: Partial<ComponentProps<typeof LoginView>> = {},
) => {
  const onLogin = vi.fn().mockResolvedValue(undefined);
  const setServerIp = vi.fn();
  render(
    <LoginView
      departments={departments}
      mode="client"
      setMode={vi.fn()}
      serverIp="127.0.0.1:3625"
      setServerIp={setServerIp}
      onLogin={onLogin}
      {...overrides}
    />,
  );
  return { onLogin, setServerIp };
};

beforeEach(() => {
  localStorage.clear();
});

describe("LoginView", () => {
  it("keeps submit disabled until every field is filled, then logs in", async () => {
    const user = userEvent.setup();
    const { onLogin } = renderLogin();

    const submit = screen.getByRole("button", { name: /enter workspace/i });
    expect(submit).toBeDisabled();

    await user.type(screen.getByPlaceholderText("Username"), "Ada");
    await user.type(screen.getByPlaceholderText("Email address"), "ada@x.com");
    await user.type(screen.getByPlaceholderText("Room password"), "pw");
    await user.selectOptions(screen.getByRole("combobox"), "1");

    expect(submit).toBeEnabled();
    await user.click(submit);
    expect(onLogin).toHaveBeenCalledWith("Ada", "ada@x.com", 1, "pw");
  });

  it("pre-fills username and email from a saved profile", () => {
    localStorage.setItem(
      "nutler.profile",
      JSON.stringify({
        username: "Saved",
        email: "saved@x.com",
        departmentId: 2,
        mode: "client",
        serverIp: "x",
      }),
    );
    renderLogin();
    expect(screen.getByPlaceholderText("Username")).toHaveValue("Saved");
    expect(screen.getByPlaceholderText("Email address")).toHaveValue(
      "saved@x.com",
    );
  });

  it("discovers LAN hosts and fills the address when one is picked", async () => {
    const user = userEvent.setup();
    const hosts: ServerInfo[] = [
      {
        address: "192.168.1.42",
        port: 3625,
        name: "Ada's host",
        user_count: 3,
      },
    ];
    const onDiscover = vi.fn().mockResolvedValue(hosts);
    const { setServerIp } = renderLogin({ onDiscover });

    await user.click(
      screen.getByRole("button", { name: /find hosts on your network/i }),
    );

    // The discovered host is listed with its real name + live count...
    expect(await screen.findByText("Ada's host")).toBeInTheDocument();
    expect(
      screen.getByText(/192\.168\.1\.42:3625 · 3 online/),
    ).toBeInTheDocument();

    // ...and picking it writes the address:port back through setServerIp.
    await user.click(screen.getByText("Ada's host"));
    expect(setServerIp).toHaveBeenCalledWith("192.168.1.42:3625");
  });

  it("shows a friendly note when no hosts are found", async () => {
    const user = userEvent.setup();
    const { setServerIp } = renderLogin({
      onDiscover: vi.fn().mockResolvedValue([]),
    });
    await user.click(
      screen.getByRole("button", { name: /find hosts on your network/i }),
    );
    expect(
      await screen.findByText(/no hosts found on your network/i),
    ).toBeInTheDocument();
    expect(setServerIp).not.toHaveBeenCalled();
  });

  it("clears a restored department that no longer exists", () => {
    localStorage.setItem(
      "nutler.profile",
      JSON.stringify({
        username: "Saved",
        email: "saved@x.com",
        departmentId: 999,
        mode: "client",
        serverIp: "x",
      }),
    );
    renderLogin();
    // 999 matches no option, so the reconcile effect resets it to the placeholder.
    const select = screen.getByRole("combobox") as HTMLSelectElement;
    expect(select.value).toBe("");
  });
});
