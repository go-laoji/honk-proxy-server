import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests/console",
  testMatch: /protocol-subscriptions\.spec\.ts/,
  outputDir: "./test-results/protocol",
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:18024",
    headless: true,
    trace: "retain-on-failure",
  },
  webServer: {
    command:
      "cargo run --locked -p hps-daemon -- --ephemeral --config tests/fixtures/console.toml --listen 127.0.0.1:18024",
    url: "http://127.0.0.1:18024/healthz",
    reuseExistingServer: false,
    timeout: 120000,
    env: {
      HPS_FORBID_HOST_NETWORK_CHANGES: "1",
      CARGO_TARGET_DIR:
        process.env.CARGO_TARGET_DIR ?? "target/protocol-acceptance",
    },
  },
});
