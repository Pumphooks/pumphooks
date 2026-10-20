import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    testTimeout: 60_000,
    // LiteSVM instances are heavy; keep files sequential.
    fileParallelism: false,
  },
});
