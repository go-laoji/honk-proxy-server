import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/console",
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:17991",
    headless: true,
    trace: "retain-on-failure",
  },
  webServer: {
    command:
      "cargo run --locked -p hps-daemon -- --config tests/fixtures/console.toml --listen 127.0.0.1:17991",
    url: "http://127.0.0.1:17991/healthz",
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
