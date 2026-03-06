import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright configuration for UI e2e tests.
 *
 * Usage:
 *   npm run e2e          # Run tests
 *   npm run e2e:ui       # Run with UI mode
 *   npm run e2e:headed   # Run in headed mode
 *   doppler run -- npx playwright test --project smoke  # Smoke tests
 */

const chromiumArgs = [
  "--no-sandbox",
  "--disable-setuid-sandbox",
  "--disable-gpu",
  "--disable-software-rasterizer",
  "--disable-dev-shm-usage",
  "--single-process",
];

// Parse proxy from environment (used by smoke tests in cloud agents).
// Proxy URL format: http://user:pass@host:port
function parseProxy(envUrl?: string) {
  if (!envUrl) return undefined;
  try {
    const url = new URL(envUrl);
    const server = `${url.protocol}//${url.hostname}:${url.port}`;
    const username = decodeURIComponent(url.username) || undefined;
    const password = decodeURIComponent(url.password) || undefined;
    return { server, username, password };
  } catch {
    return { server: envUrl };
  }
}
const proxy = parseProxy(process.env.HTTPS_PROXY || process.env.https_proxy);

export default defineConfig({
  testDir: "./e2e",
  outputDir: "./e2e/test-results",

  // Run tests in parallel
  fullyParallel: true,

  // Fail the build on CI if you accidentally left test.only in the source code
  forbidOnly: !!process.env.CI,

  // Retry on CI only
  retries: process.env.CI ? 2 : 0,

  // Opt out of parallel tests on CI
  workers: process.env.CI ? 1 : undefined,

  // Reporter to use
  reporter: [["html", { outputFolder: "./e2e/playwright-report" }], ["list"]],

  // Shared settings for all projects
  use: {
    // Base URL to use in actions like `await page.goto('/')`
    baseURL: "http://localhost:9100",

    // Collect trace when retrying the failed test
    trace: "on-first-retry",

    // Take screenshot on failure
    screenshot: "only-on-failure",
  },

  // Configure projects for browsers
  projects: [
    {
      name: "chromium",
      testIgnore: "smoke.spec.ts",
      use: {
        ...devices["Desktop Chrome"],
        launchOptions: {
          executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH,
          args: chromiumArgs,
        },
      },
    },
    // Smoke tests against deployed environments (e.g., dev.everruns.com)
    // Run: doppler run -- npx playwright test --project smoke
    {
      name: "smoke",
      testMatch: "smoke.spec.ts",
      use: {
        ...devices["Desktop Chrome"],
        baseURL: process.env.SMOKE_BASE_URL || "https://dev.everruns.com",
        // Forward proxy from environment so tests work in cloud agents
        ...(proxy ? { proxy } : {}),
        // Accept TLS certs from proxy/CDN in non-local environments
        ignoreHTTPSErrors: true,
        launchOptions: {
          executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH,
          args: chromiumArgs,
        },
      },
    },
  ],

  // Run local dev server before starting tests (only for non-smoke projects)
  webServer: {
    command: "npm run dev",
    url: "http://localhost:9100",
    reuseExistingServer: !process.env.CI,
    timeout: 120 * 1000,
  },
});
