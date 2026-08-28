import { defineConfig } from "vitest/config";

// No react plugin: esbuild compiles JSX from tsconfig's `jsx: react-jsx`, the
// same way apps/desktop's config does. No coverage block either — the root
// vitest.config.ts owns the gate, because the floors are measured across every
// package together.
export default defineConfig({
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
  },
});
