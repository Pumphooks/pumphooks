import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: {
      "@pump-hooks/execution-lab": fileURLToPath(new URL("../execution-lab/src/index.ts", import.meta.url)),
      "@pump-hooks/sdk": fileURLToPath(new URL("../sdk/src/index.ts", import.meta.url)),
      "@pump-hooks/hook-security": fileURLToPath(new URL("../hook-security/src/index.ts", import.meta.url)),
    },
  },
  test: {
    include: ["test/**/*.test.ts"],
    testTimeout: 300_000,
    fileParallelism: false,
  },
});
