import { chromium } from "@playwright/test";

const baseURL = process.env.BENCHMARK_BASE_URL ?? "http://localhost:9100";
const orgId = "org_00000000000000000000000000000001";
const targets = [
  { name: "Sessions", pathname: "/sessions", heading: "Sessions" },
  { name: "Agents", pathname: "/agents", heading: "Agents" },
  { name: "Settings", pathname: "/settings/organization", heading: "Organization" },
];

function percentile(samples, quantile) {
  const sorted = [...samples].sort((a, b) => a - b);
  return sorted[Math.ceil(quantile * sorted.length) - 1];
}

function summarize(samples) {
  return {
    samples_ms: samples,
    median_ms: percentile(samples, 0.5),
    p95_ms: percentile(samples, 0.95),
  };
}

async function mockApi(page) {
  await page.route("**/api/v1/**", async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    let json;

    if (pathname === "/api/v1/auth/config") {
      json = {
        mode: "full",
        password_auth_enabled: true,
        oauth_providers: [],
        signup_enabled: true,
      };
    } else if (pathname === "/api/v1/auth/me") {
      json = {
        id: "user-1",
        email: "benchmark@example.com",
        name: "Benchmark User",
        roles: [],
        email_verified: true,
        organizations: [{ public_id: orgId, name: "Benchmark Org", role: "owner" }],
      };
    } else if (pathname === `/api/v1/orgs/${orgId}`) {
      json = {
        id: orgId,
        public_id: orgId,
        name: "Benchmark Org",
        role: "owner",
        onboarding_completed_at: "2026-07-15T00:00:00Z",
      };
    } else if (pathname.endsWith("/feature-flags") || pathname === "/api/v1/feature-flags") {
      json = {
        global_chat: false,
        notifications: false,
        evals: false,
        app_budgets: false,
        agent_versions: false,
        voice: false,
        agent_delegation: false,
        observers: false,
        public_chat: false,
      };
    } else if (pathname.endsWith("/switch-org")) {
      json = { success: true, org_id: orgId };
    } else if (pathname.endsWith("/config")) {
      json = { policies: {} };
    } else {
      json = { data: [], has_more: false, total: 0 };
    }

    await route.fulfill({ json });
  });
}

async function waitForReady(page, heading) {
  await page.getByRole("heading", { name: heading, exact: true }).first().waitFor();
  await page.waitForLoadState("networkidle");
}

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({ viewport: { width: 1440, height: 2000 } });
await context.addCookies([
  {
    name: "access_token",
    value: "benchmark-only",
    url: baseURL,
  },
]);
const page = await context.newPage();
await mockApi(page);

const startupRequests = [];
page.on("request", (request) => startupRequests.push(request));
const dashboardStarted = performance.now();
await page.goto(`${baseURL}/dashboard`, { waitUntil: "domcontentloaded" });
await waitForReady(page, "Dashboard");
const dashboardReadinessMs = Math.round(performance.now() - dashboardStarted);

const measuredRequests = startupRequests.filter((request) => {
  const url = new URL(request.url());
  return (
    url.pathname.startsWith("/api/v1/") ||
    request.resourceType() === "document" ||
    request.headers().rsc === "1"
  );
});
const pageRequests = measuredRequests.filter(
  (request) => request.resourceType() === "document" || request.headers().rsc === "1",
);
const apiRequests = measuredRequests.filter((request) =>
  new URL(request.url()).pathname.startsWith("/api/v1/"),
);
const uniqueRoutes = new Set(measuredRequests.map((request) => new URL(request.url()).pathname));

const navigation = {};
for (const target of targets) {
  const samples = [];
  for (let sample = 0; sample < 5; sample += 1) {
    await page.goto(`${baseURL}/dashboard`, { waitUntil: "domcontentloaded" });
    await waitForReady(page, "Dashboard");
    const link = page.getByRole("link", { name: target.name, exact: true });
    if ((await link.count()) === 0) {
      throw new Error(
        `Missing ${target.name} sidebar link at ${page.url()}; links=${JSON.stringify(await page.getByRole("link").allTextContents())}`,
      );
    }
    await link.waitFor();
    await link.hover();
    await page.waitForTimeout(150);
    const started = performance.now();
    await link.click();
    await page.waitForURL((url) => url.pathname === target.pathname);
    await waitForReady(page, target.heading);
    samples.push(Math.round(performance.now() - started));
  }
  navigation[target.name] = summarize(samples);
}

console.log(
  JSON.stringify(
    {
      environment: baseURL,
      methodology:
        "Chromium 1440x2000; mocked authenticated user/API; DOM heading plus network idle; 150ms hover intent before click",
      dashboard: {
        total_requests: measuredRequests.length,
        page_rsc_requests: pageRequests.length,
        api_requests: apiRequests.length,
        unique_routes: uniqueRoutes.size,
        readiness_ms: dashboardReadinessMs,
        routes: [...uniqueRoutes].sort(),
      },
      navigation,
    },
    null,
    2,
  ),
);

await browser.close();
