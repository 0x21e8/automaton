import { defineConfig, devices } from "playwright/test";

export default defineConfig({
  testDir: "./tests/ui",
  fullyParallel: true,
  reporter: "line",
  use: {
    baseURL: "http://127.0.0.1:5173",
    trace: "on-first-retry"
  },
  webServer: [
    {
      command: "npm run dev:web",
      url: "http://127.0.0.1:5173",
      reuseExistingServer: true,
      timeout: 120_000
    },
    {
      command: "npm run dev:evaluator-web",
      url: "http://127.0.0.1:4173",
      reuseExistingServer: true,
      timeout: 120_000
    }
  ],
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } } },
    { name: "tablet", use: { ...devices["Desktop Chrome"], viewport: { width: 768, height: 1024 } } },
    { name: "mobile", use: { ...devices["Desktop Chrome"], viewport: { width: 390, height: 844 }, isMobile: true } }
  ]
});
