import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SettingsModal } from "./SettingsModal";
import { Preferences } from "../preferences";

const prefs: Preferences = { notifications: "all", sendOnEnter: true };

const renderSettings = (overrides: Partial<Preferences> = {}) => {
  const onToggleTheme = vi.fn();
  const onSetPreferences = vi.fn();
  const onClose = vi.fn();
  render(
    <SettingsModal
      theme="dark"
      onToggleTheme={onToggleTheme}
      preferences={{ ...prefs, ...overrides }}
      onSetPreferences={onSetPreferences}
      onClose={onClose}
    />,
  );
  return { onToggleTheme, onSetPreferences, onClose };
};

describe("SettingsModal", () => {
  it("reflects the current notification level and switches it", async () => {
    const user = userEvent.setup();
    const { onSetPreferences } = renderSettings();

    expect(screen.getByRole("radio", { name: /all messages/i })).toBeChecked();
    await user.click(screen.getByRole("radio", { name: /mentions only/i }));
    expect(onSetPreferences).toHaveBeenCalledWith({
      notifications: "mentions",
    });
  });

  it("toggles the theme by selecting the inactive option", async () => {
    const user = userEvent.setup();
    const { onToggleTheme } = renderSettings();
    // We're in dark mode; picking Light toggles.
    await user.click(screen.getByRole("radio", { name: /light/i }));
    expect(onToggleTheme).toHaveBeenCalledTimes(1);
    // Picking the already-active Dark option does nothing.
    await user.click(screen.getByRole("radio", { name: /^dark$/i }));
    expect(onToggleTheme).toHaveBeenCalledTimes(1);
  });

  it("flips send-on-Enter and reflects the switch state", async () => {
    const user = userEvent.setup();
    const { onSetPreferences } = renderSettings({ sendOnEnter: true });
    const sw = screen.getByRole("switch", { name: /send with enter/i });
    expect(sw).toBeChecked();
    await user.click(sw);
    expect(onSetPreferences).toHaveBeenCalledWith({ sendOnEnter: false });
  });

  it("closes from the header button", async () => {
    const user = userEvent.setup();
    const { onClose } = renderSettings();
    await user.click(screen.getByRole("button", { name: /close/i }));
    expect(onClose).toHaveBeenCalled();
  });
});
