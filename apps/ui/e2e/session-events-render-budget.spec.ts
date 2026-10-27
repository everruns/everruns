import { expect, test } from "@playwright/test";

const ORG = "org_00000000000000000000000000000001";
const SESSION = "session_00000000000000000000000000000001";
test.use({ deviceScaleFactor: 2, viewport: { width: 1440, height: 900 } });

test("keeps screenshot-heavy Events readable with bounded browser layout", async ({
  page,
  context,
  baseURL,
}, testInfo) => {
  test.setTimeout(60_000);
  const screenshot = "A".repeat(64 * 1024);
  const events = Array.from({ length: 445 }, (_, sequence) => ({
    id: `fixture-${sequence}`,
    session_id: SESSION,
    sequence,
    type: "tool.completed",
    ts: "2026-10-26T00:00:00Z",
    data: {
      tool_call_id: `call-${sequence}`,
      tool_name: "desktop",
      result: sequence >= 395 ? { screenshot } : { status: "ok" },
    },
  }));
  await context.addCookies([
    { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
  ]);
  // No live subscription, remote provider, controller or production endpoint.
  await page.addInitScript(() => {
    class IdleStream extends EventTarget {
      close() {}
      onopen = null;
      onerror = null;
    }
    Object.defineProperty(window, "EventSource", { value: IdleStream });
  });
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    let json: unknown = { data: [], has_more: false, total: 0 };
    if (path === "/api/v1/auth/config")
      json = {
        mode: "none",
        password_auth_enabled: false,
        oauth_providers: [],
        signup_enabled: false,
      };
    else if (path === "/api/v1/auth/me")
      json = {
        id: "fixture-user",
        email: "fixture@example.test",
        name: "Fixture",
        roles: [],
        email_verified: true,
        organizations: [{ public_id: ORG, name: "Fixture", role: "owner" }],
      };
    else if (path.endsWith("/feature-flags")) json = {};
    else if (path.endsWith("/switch-org")) json = { success: true, org_id: ORG };
    else if (path === `/api/v1/sessions/${SESSION}`)
      json = {
        id: SESSION,
        organization_id: ORG,
        harness_id: "harness-fixture",
        agent_id: null,
        title: "Bounded Events fixture",
        tags: [],
        features: [],
        model_id: null,
        status: "idle",
        created_at: "2026-10-26T00:00:00Z",
        updated_at: "2026-10-26T00:00:00Z",
      };
    else if (path === `/api/v1/sessions/${SESSION}/resolved-model`) json = { model_id: null };
    else if (path === `/api/v1/sessions/${SESSION}/events`)
      json = { data: events, has_more: false, total: events.length };
    else if (path === `/api/v1/orgs/${ORG}`)
      json = { id: ORG, name: "Fixture", default_model_id: null };
    await route.fulfill({ json });
  });
  await page.goto(`/sessions/${SESSION}/events`);
  const previews = page.locator("tbody pre");
  await expect(previews).toHaveCount(50);
  const bounded = await previews.allTextContents();
  expect(bounded.every((text) => text.length <= 8192)).toBe(true);

  const cdp = await context.newCDPSession(page);
  await cdp.send("Performance.enable");
  const measure = async (texts: string[]) => {
    await cdp.send("HeapProfiler.collectGarbage");
    const before = await cdp.send("Performance.getMetrics");
    const elapsedMs = await page.evaluate(async (values) => {
      const start = performance.now();
      document.querySelectorAll("tbody pre").forEach((node, i) => {
        node.textContent = values[i];
      });
      // Include the wrapped-text layout which jsdom cannot measure.
      void document.body.offsetHeight;
      await new Promise(requestAnimationFrame);
      await new Promise(requestAnimationFrame);
      return performance.now() - start;
    }, texts);
    const after = await cdp.send("Performance.getMetrics");
    await cdp.send("HeapProfiler.collectGarbage");
    const retained = await cdp.send("Performance.getMetrics");
    const liveCharacters = await previews.evaluateAll((nodes) =>
      nodes.reduce((sum, node) => sum + (node.textContent?.length ?? 0), 0),
    );
    const metric = (metrics: typeof before.metrics, name: string) =>
      metrics.find((m) => m.name === name)?.value ?? 0;
    return {
      elapsedMs,
      characters: texts.reduce((n, t) => n + t.length, 0),
      liveCharacters,
      taskSeconds: metric(after.metrics, "TaskDuration") - metric(before.metrics, "TaskDuration"),
      layoutSeconds:
        metric(after.metrics, "LayoutDuration") - metric(before.metrics, "LayoutDuration"),
      heapUsedBytesAfterGC: metric(retained.metrics, "JSHeapUsedSize"),
    };
  };
  // Fixed, capped control reproduces the former raw-pre behavior on the same
  // DOM/CSS. It cannot replay agent actions or consume provider credits.
  const legacy = await measure(
    events
      .slice(-50)
      .reverse()
      .map((event) => JSON.stringify(event, null, 2)),
  );
  await testInfo.attach("legacy-events.png", {
    body: await page.screenshot(),
    contentType: "image/png",
  });
  const fixed = await measure(bounded);
  await testInfo.attach("bounded-events.png", {
    body: await page.screenshot(),
    contentType: "image/png",
  });
  await testInfo.attach("events-browser-before-after.json", {
    body: JSON.stringify(
      {
        fixture: "445 synthetic events; 50 visible; 64KiB screenshot each",
        legacy,
        fixed,
        limitations:
          "No DevTools or controller attached; not proof of the historical desktop cause. Heap includes the common fixture and app.",
      },
      null,
      2,
    ),
    contentType: "application/json",
  });
  console.info("events-browser-before-after", JSON.stringify({ legacy, fixed }));
  expect(legacy.liveCharacters).toBe(legacy.characters);
  expect(fixed.liveCharacters).toBe(fixed.characters);
  expect(fixed.characters).toBeLessThan(legacy.characters / 20);
  await expect(page.getByRole("button", { name: "Next page" })).toBeEnabled();
  await page.getByRole("button", { name: "Next page" }).click();
  await expect(page.getByText("Showing 51-100 of 445 events")).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("bounded-events.png") });
});
