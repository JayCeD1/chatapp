import React from "react";
import { X, Sun, Moon, Bell } from "lucide-react";
import { Theme } from "../hooks/useTheme";
import { Preferences, NotificationMode } from "../preferences";
import { useFocusTrap } from "../hooks/useFocusTrap";

interface SettingsModalProps {
  theme: Theme;
  onToggleTheme: () => void;
  preferences: Preferences;
  onSetPreferences: (patch: Partial<Preferences>) => void;
  onClose: () => void;
}

const notificationOptions: {
  value: NotificationMode;
  label: string;
  hint: string;
}[] = [
  {
    value: "all",
    label: "All messages",
    hint: "Notify when the window isn't focused, and on @mentions",
  },
  {
    value: "mentions",
    label: "Mentions only",
    hint: "Notify only when someone @mentions you",
  },
  { value: "off", label: "Off", hint: "Never show desktop notifications" },
];

export const SettingsModal: React.FC<SettingsModalProps> = ({
  theme,
  onToggleTheme,
  preferences,
  onSetPreferences,
  onClose,
}) => {
  const trapRef = useFocusTrap<HTMLDivElement>(onClose);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onMouseDown={onClose}
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
    >
      <div
        ref={trapRef}
        className="w-full max-w-md bg-[var(--surface)] border border-[var(--border)] rounded-2xl shadow-2xl animate-scale-in overflow-hidden"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 h-14 border-b border-[var(--border)]">
          <h2 className="font-semibold text-[var(--text)]">Settings</h2>
          <button
            onClick={onClose}
            aria-label="Close"
            className="p-1.5 rounded-md text-[var(--text-faint)] hover:text-[var(--text)] hover:bg-[var(--surface-2)] transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="px-5 py-4 space-y-6 max-h-[70vh] overflow-y-auto scrollbar-thin scrollbar-track-transparent">
          {/* Appearance */}
          <section>
            <h3 className="text-[11px] font-semibold uppercase tracking-wider text-[var(--text-faint)] mb-2">
              Appearance
            </h3>
            <div
              className="bg-[var(--surface-2)] p-1 rounded-xl flex"
              role="group"
              aria-label="Theme"
            >
              <ThemeOption
                icon={<Sun className="w-4 h-4" />}
                label="Light"
                active={theme === "light"}
                onClick={() => theme !== "light" && onToggleTheme()}
              />
              <ThemeOption
                icon={<Moon className="w-4 h-4" />}
                label="Dark"
                active={theme === "dark"}
                onClick={() => theme !== "dark" && onToggleTheme()}
              />
            </div>
          </section>

          {/* Notifications */}
          <section>
            <h3 className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-[var(--text-faint)] mb-2">
              <Bell className="w-3.5 h-3.5" /> Desktop notifications
            </h3>
            <div
              className="space-y-1"
              role="radiogroup"
              aria-label="Desktop notifications"
            >
              {notificationOptions.map((opt) => {
                const selected = preferences.notifications === opt.value;
                return (
                  <button
                    key={opt.value}
                    type="button"
                    role="radio"
                    aria-checked={selected}
                    onClick={() =>
                      onSetPreferences({ notifications: opt.value })
                    }
                    className={`w-full flex items-start gap-3 px-3 py-2 rounded-lg text-left transition-colors ${
                      selected
                        ? "bg-[var(--accent-soft)] border border-[var(--accent)]"
                        : "border border-transparent hover:bg-[var(--surface-2)]"
                    }`}
                  >
                    <span
                      className={`mt-0.5 flex items-center justify-center w-4 h-4 rounded-full border shrink-0 ${
                        selected
                          ? "border-[var(--accent)]"
                          : "border-[var(--border)]"
                      }`}
                    >
                      {selected && (
                        <span className="w-2 h-2 rounded-full bg-[var(--accent)]" />
                      )}
                    </span>
                    <span className="min-w-0">
                      <span className="block text-sm text-[var(--text)]">
                        {opt.label}
                      </span>
                      <span className="block text-[11px] text-[var(--text-faint)]">
                        {opt.hint}
                      </span>
                    </span>
                  </button>
                );
              })}
            </div>
          </section>

          {/* Composer */}
          <section>
            <h3 className="text-[11px] font-semibold uppercase tracking-wider text-[var(--text-faint)] mb-2">
              Composer
            </h3>
            <div className="flex items-center justify-between gap-3">
              <label htmlFor="send-on-enter" className="min-w-0">
                <span className="block text-sm text-[var(--text)]">
                  Send with Enter
                </span>
                <span className="block text-[11px] text-[var(--text-faint)]">
                  {preferences.sendOnEnter
                    ? "Enter sends. Cmd/Ctrl+Enter also sends."
                    : "Enter won't send — use the Send button or Cmd/Ctrl+Enter."}
                </span>
              </label>
              <button
                id="send-on-enter"
                type="button"
                role="switch"
                aria-checked={preferences.sendOnEnter}
                onClick={() =>
                  onSetPreferences({ sendOnEnter: !preferences.sendOnEnter })
                }
                className={`relative w-10 h-6 rounded-full shrink-0 transition-colors ${
                  preferences.sendOnEnter
                    ? "bg-[var(--accent)]"
                    : "bg-[var(--surface-3)]"
                }`}
              >
                <span
                  className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform ${
                    preferences.sendOnEnter ? "translate-x-4" : ""
                  }`}
                />
              </button>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
};

const ThemeOption: React.FC<{
  icon: React.ReactNode;
  label: string;
  active: boolean;
  onClick: () => void;
}> = ({ icon, label, active, onClick }) => (
  <button
    type="button"
    aria-pressed={active}
    onClick={onClick}
    className={`flex-1 flex items-center justify-center gap-1.5 py-2 rounded-lg text-sm font-medium transition-colors ${
      active
        ? "bg-[var(--surface-3)] text-[var(--text)]"
        : "text-[var(--text-dim)] hover:text-[var(--text)]"
    }`}
  >
    {icon}
    {label}
  </button>
);
