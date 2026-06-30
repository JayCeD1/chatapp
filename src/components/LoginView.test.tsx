import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LoginView } from "./LoginView";
import { Department } from "../types";

const departments: Department[] = [
  { id: 1, name: "Engineering" },
  { id: 2, name: "Design" },
];

const renderLogin = () => {
  const onLogin = vi.fn().mockResolvedValue(undefined);
  render(
    <LoginView
      departments={departments}
      mode="client"
      setMode={vi.fn()}
      serverIp="127.0.0.1:3625"
      setServerIp={vi.fn()}
      onLogin={onLogin}
    />,
  );
  return { onLogin };
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
