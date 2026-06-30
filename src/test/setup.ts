// Registers @testing-library/jest-dom matchers (toBeInTheDocument, etc.) on Vitest's
// `expect`, including their TypeScript augmentation.
import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// We don't run with `globals: true`, so RTL can't auto-register its cleanup — unmount
// rendered trees after each test ourselves to keep queries unambiguous.
afterEach(() => cleanup());
