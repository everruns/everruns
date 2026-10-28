import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./e2e",
  workers: 1,
  timeout: 90000,
  use: {
    browserName: "chromium",
    deviceScaleFactor: 2,
    colorScheme: "light",
    viewport: { width: 1440, height: 1000 },
  },
  webServer: [
    { command: "pnpm exec tsx e2e/fixture-api.ts", port: 9141, reuseExistingServer: false },
    {
      command: "pnpm exec next dev --port 9140",
      port: 9140,
      reuseExistingServer: false,
      env: {
        CHAT_DOMAIN: "localhost",
        CHAT_API_URL: "http://127.0.0.1:9141",
        CHAT_OIDC_ISSUER: "https://id.example.test",
        CHAT_OIDC_CLIENT_ID: "render-fixture",
        CHAT_OIDC_CLIENT_SECRET: "render-fixture",
        CHAT_SESSION_KEY: "00".repeat(32),
        CHAT_DATABASE_URL: "postgres://render-fixture@127.0.0.1/render-fixture",
      },
    },
  ],
});
