import { useEffect, useRef } from "react";

// Accessibility helper for modal dialogs (WCAG 2.1.1 / 2.4.3):
//  - moves focus into the dialog on open,
//  - keeps Tab / Shift+Tab cycling within it,
//  - closes on Escape,
//  - restores focus to the previously-focused element on close.
// Attach the returned ref to the dialog's content container.
export function useFocusTrap<T extends HTMLElement>(onClose: () => void) {
  const ref = useRef<T>(null);
  // Keep the latest onClose without re-running the effect (which would re-steal focus
  // on every parent re-render / keystroke).
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  // Capture the element to restore on close at RENDER time — before the dialog commits
  // and moves focus inward. Capturing inside the effect would record the dialog's own
  // first field (focus has already moved by then), so close would drop focus to <body>.
  const restoreTo = useRef<HTMLElement | null>(null);
  if (restoreTo.current === null) {
    restoreTo.current = document.activeElement as HTMLElement | null;
  }

  useEffect(() => {
    const node = ref.current;
    const previouslyFocused = restoreTo.current;
    const selector =
      'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';
    const focusable = () =>
      node
        ? Array.from(node.querySelectorAll<HTMLElement>(selector)).filter(
            (el) => el.offsetParent !== null,
          )
        : [];

    // Focus the first field when the dialog opens.
    (focusable()[0] ?? node)?.focus();

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onCloseRef.current();
        return;
      }
      if (e.key !== "Tab") return;
      const els = focusable();
      if (els.length === 0) return;
      const first = els[0];
      const last = els[els.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      previouslyFocused?.focus?.();
    };
  }, []);

  return ref;
}
