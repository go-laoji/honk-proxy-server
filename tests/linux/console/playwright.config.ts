import { defineConfig } from "@playwright/test";
const baseURL = process.env.HPS_LINUX_UI_ENDPOINT;
if (!baseURL)
  throw new Error(
    "HPS_LINUX_UI_ENDPOINT must name the isolated Linux validation service",
  );
export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  workers: 1,
  fullyParallel: false,
  timeout: 60000,
  use: { baseURL, headless: true, trace: "retain-on-failure" },
});
