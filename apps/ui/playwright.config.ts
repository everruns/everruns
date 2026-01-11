import { defineConfig, devices } from '@playwright/test';

/**
 * Playwright configuration for UI e2e tests.
 *
 * Usage:
 *   npm run e2e          # Run tests
 *   npm run e2e:ui       # Run with UI mode
 *   npm run e2e:headed   # Run in headed mode
 */
export default defineConfig({
  testDir: './e2e',
  outputDir: './e2e/test-results',

  // Run tests in parallel
  fullyParallel: true,

  // Fail the build on CI if you accidentally left test.only in the source code
  forbidOnly: !!process.env.CI,

  // Retry on CI only
  retries: process.env.CI ? 2 : 0,

  // Opt out of parallel tests on CI
  workers: process.env.CI ? 1 : undefined,

  // Reporter to use
  reporter: [
    ['html', { outputFolder: './e2e/playwright-report' }],
    ['list'],
  ],

  // Shared settings for all projects
  use: {
    // Base URL to use in actions like `await page.goto('/')`
    baseURL: 'http://localhost:9100',

    // Collect trace when retrying the failed test
    trace: 'on-first-retry',

    // Take screenshot on failure
    screenshot: 'only-on-failure',
  },

  // Configure projects for browsers
  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        // Use older chromium that works in restricted environments
        launchOptions: {
          executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH,
          args: [
            '--no-sandbox',
            '--disable-setuid-sandbox',
            '--disable-gpu',
            '--disable-software-rasterizer',
            '--disable-dev-shm-usage',
            '--single-process',
          ],
        },
      },
    },
  ],

  // Run local dev server before starting tests
  webServer: {
    command: 'npm run dev',
    url: 'http://localhost:9100',
    reuseExistingServer: !process.env.CI,
    timeout: 120 * 1000,
  },
});
