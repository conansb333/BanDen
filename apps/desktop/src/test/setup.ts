import "@testing-library/jest-dom/vitest";

// The app talks to Tauri at runtime; tests must not depend on it.
if (!("__TAURI_INTERNALS__" in window)) {
  (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
    invoke: () => Promise.reject(new Error("invoke not available in tests")),
    metadata: { currentWindow: { label: "test" }, currentWebview: { label: "test" } },
  };
}
