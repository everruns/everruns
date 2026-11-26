import { expect, test, type Page } from "@playwright/test";
import { mockKnowledgeApi } from "./fixtures/knowledge-app";

// The Trace tab's Turns minimap: clicking a cell must bring that range of
// turns into view in the trace, in a session far longer than one loaded page.

const ORG = "org_00000000000000000000000000000001";
const SESSION = "session_00000000000000000000000000000002";
const TURN_COUNT = 1851;
const BUCKET = 16;
test.use({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 });

function turn(n: number) {
  const started = new Date(Date.UTC(2026, 9, 1) + n * 60_000).toISOString();
  const step = (i: number, kind: string) => ({
    type: "step",
    turn: n,
    step: i,
    kind,
    name: kind === "tool" ? "bash" : "gpt-6.1-sol",
    status: "ok",
    offset_ms: i * 1000,
    duration_ms: 900,
    start_sequence: n * 10 + i,
    end_sequence: n * 10 + i,
    started_at: started,
  });
  return {
    turn: n,
    turn_id: `turn-${n}`,
    prompt: `Prompt of turn ${n}`,
    status: "completed",
    started_at: started,
    ended_at: started,
    duration_ms: 3000,
    start_sequence: n * 10,
    end_sequence: n * 100 + 30,
    error_count: 0,
    input_tokens: 100,
    output_tokens: 20,
    model_calls: 15,
    tool_calls: 15,
    subagent_calls: 0,
    step_count: 30,
    // Tall turns: a far jump replaces the loaded window with one of a very
    // different height, which is what used to leave the reader in place.
    items: Array.from({ length: 30 }, (_, i) => ({
      ...step(i + 1, i % 2 ? "tool" : "model"),
      narration: i % 3 ? null : "Reading the repository before the next change. ".repeat(4),
    })),
  };
}

function turnsPage(params: URLSearchParams) {
  const limit = Number(params.get("limit") ?? 20);
  let from: number;
  let to: number;
  if (params.has("before")) {
    const before = Number(params.get("before"));
    from = Math.max(1, before - limit);
    to = before - 1;
  } else if (params.has("after")) {
    from = Number(params.get("after")) + 1;
    to = from + limit - 1;
  } else if (params.has("around")) {
    from = Math.max(1, Number(params.get("around")) - Math.floor(limit / 2));
    to = from + limit - 1;
  } else {
    from = Math.max(1, TURN_COUNT - limit + 1);
    to = TURN_COUNT;
  }
  to = Math.min(to, TURN_COUNT);
  const turns = [];
  for (let n = from; n <= to; n += 1) turns.push(turn(n));
  return { turns, turn_count: TURN_COUNT, has_earlier: from > 1, has_later: to < TURN_COUNT };
}

async function mockSession(page: Page, baseURL: string, withChat = false) {
  await page
    .context()
    .addCookies([
      { name: "access_token", value: "e2e-only", domain: new URL(baseURL).hostname, path: "/" },
    ]);
  await page.addInitScript(() => {
    class IdleStream extends EventTarget {
      close() {}
      onopen = null;
      onerror = null;
    }
    Object.defineProperty(window, "EventSource", { value: IdleStream });
  });
  const buckets = [];
  for (let from = 1; from <= TURN_COUNT; from += BUCKET) {
    buckets.push({
      from_turn: from,
      to_turn: Math.min(TURN_COUNT, from + BUCKET - 1),
      steps: 48,
      errors: 0,
      duration_ms: 48_000,
    });
  }
  await page.route("**/api/v1/**", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    let json: unknown = { data: [], has_more: false, total: 0 };
    if (path === "/api/v1/platform/auth/config")
      json = {
        mode: "none",
        password_auth_enabled: false,
        oauth_providers: [],
        signup_enabled: false,
      };
    else if (path === "/api/v1/platform/auth/me")
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
    else if (path === `/api/v1/platform/sessions/${SESSION}`)
      json = {
        id: SESSION,
        organization_id: ORG,
        harness_id: "harness-fixture",
        agent_id: null,
        title: "Long trace fixture",
        tags: [],
        features: [],
        model_id: null,
        status: "idle",
        created_at: "2026-10-01T00:00:00Z",
        updated_at: "2026-10-01T00:00:00Z",
      };
    else if (path === `/api/v1/platform/sessions/${SESSION}/resolved-model`)
      json = { model_id: null };
    else if (path === `/api/v1/platform/sessions/${SESSION}/trace`)
      json = {
        turn_count: TURN_COUNT,
        step_count: TURN_COUNT * 30,
        error_count: 0,
        error_turns: [],
        input_tokens: 0,
        output_tokens: 0,
        bucket_size: BUCKET,
        buckets,
      };
    else if (path === `/api/v1/platform/sessions/${SESSION}/trace/turns`)
      json = turnsPage(url.searchParams);
    else if (path.startsWith(`/api/v1/platform/sessions/${SESSION}/trace/turns/`)) {
      // turns/{turn}/steps/{step}
      const [turnNo, , stepNo] = path.split("/trace/turns/")[1]!.split("/");
      const detail = turn(Number(turnNo)).items[Number(stepNo) - 1];
      json = path.endsWith("/events")
        ? { events: [] }
        : { step: detail, events: [], input: null, output: null, request: null };
    } else if (path === `/api/v1/platform/orgs/${ORG}`)
      json = { id: ORG, name: "Fixture", default_model_id: null };
    else if (withChat) return route.fallback();
    await route.fulfill({ json });
  });
}

/** The trace list's scroll container. */
function traceList(page: Page) {
  return page.locator('[data-trace-list="true"]');
}

/** True when the turn's header sits inside the visible part of the trace list. */
async function headerInView(page: Page, turnNo: number) {
  const header = traceList(page).getByText(`Prompt of turn ${turnNo}`, { exact: true });
  const list = await traceList(page).boundingBox();
  const box = await header.boundingBox();
  if (!list || !box) return false;
  return box.y >= list.y - 1 && box.y + box.height <= list.y + list.height + 1;
}

test("a minimap cell brings its turns into view", async ({ page, baseURL }) => {
  test.setTimeout(90_000);
  await mockSession(page, baseURL!);
  await page.goto(`/sessions/${SESSION}/trace?step=1833.1`);

  const minimap = page.getByRole("list", { name: "Session minimap" });
  await expect(minimap).toBeVisible({ timeout: 30_000 });
  // Opens at the linked step.
  await expect.poll(() => headerInView(page, 1833)).toBe(true);

  // Far away: turns 33–48 replace the loaded window and scroll into view.
  await minimap.getByRole("listitem", { name: /^Turns 33 to 48/ }).click();
  await expect.poll(() => headerInView(page, 33), { timeout: 10_000 }).toBe(true);
  await expect(minimap.getByRole("listitem", { name: /^Turns 33 to 48/ })).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(minimap.getByRole("listitem", { name: /^Turns 1825 to 1840/ })).not.toHaveAttribute(
    "aria-current",
    "true",
  );

  // Next door: turns 49–64 grow the window and scroll into view.
  await minimap.getByRole("listitem", { name: /^Turns 49 to 64/ }).click();
  await expect.poll(() => headerInView(page, 49), { timeout: 10_000 }).toBe(true);

  // Back to the end.
  await minimap.getByRole("listitem", { name: /^Turns 1841 to 1851/ }).click();
  await expect.poll(() => headerInView(page, 1841), { timeout: 10_000 }).toBe(true);
});

test("opens the requested message page or tools in a fixed-size model request modal", async ({
  page,
  baseURL,
}) => {
  test.setTimeout(90_000);
  await mockSession(page, baseURL!);
  const call = `**/api/v1/platform/sessions/${SESSION}/trace/turns/1833/steps/1`;
  const preview = "Inspect the attached report";
  const system = "Read workspace reports before answering.";
  const longMessage = `${preview}\n${"A long line in the report.\n".repeat(150)}`;
  const messages = Array.from({ length: 65 }, (_, index) => ({
    index,
    role: index === 0 ? "system" : "user",
    preview: index === 0 ? system : index === 62 ? preview : `Message ${index}`,
    size_bytes: index === 62 ? longMessage.length : 20,
    content: {
      content: [
        {
          type: "text",
          text: index === 0 ? system : index === 62 ? longMessage : `Message ${index}`,
        },
      ],
    },
  }));
  await page.route(call, (route) =>
    route.fulfill({
      json: {
        step: turn(1833).items[0],
        events: [],
        request: {
          message_count: messages.length,
          new_from: 62,
          tool_count: 1,
          system_preview: system,
          system_index: 0,
          new_messages: [messages[62]],
        },
      },
    }),
  );
  await page.route(`${call}/request?*`, (route) => {
    const params = new URL(route.request().url()).searchParams;
    const offset = Number(params.get("offset"));
    const limit = Number(params.get("limit"));
    return route.fulfill({
      json: {
        message_count: messages.length,
        new_from: 62,
        has_native_request: false,
        tools: offset === 0 ? [{ name: "read_file", description: "Read a workspace file" }] : [],
        messages: messages.slice(offset, offset + limit),
      },
    });
  });
  await page.goto(`/sessions/${SESSION}/trace?step=1833.1`);
  const toolsLink = page.getByRole("button", { name: /View 1 tools/ });
  await expect(toolsLink).toBeVisible({ timeout: 30_000 });
  await toolsLink.click();
  const modal = page.getByRole("dialog", { name: "Model request" });
  await expect(modal.getByText("Read a workspace file")).toBeVisible();
  await expect.poll(async () => (await modal.boundingBox())?.height).toBe(868);
  const bounds = await modal.boundingBox();
  expect(bounds?.width).toBeGreaterThan(1300);
  await modal.getByRole("button", { name: "Close" }).click();
  await expect(modal).toBeHidden();

  await page.getByRole("button", { name: new RegExp(system) }).click();
  await expect(modal.getByRole("button", { name: /^0 system/ })).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await modal.getByRole("button", { name: "Close" }).click();

  await page.getByRole("button", { name: new RegExp(preview) }).click();
  await expect(modal.getByRole("button", { name: /^62 user/ })).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await expect(modal.getByText(/A long line in the report/)).toBeVisible();
  await expect(modal.getByText(preview, { exact: true })).toHaveCount(0);
  await expect(modal.getByRole("button", { name: "Load earlier messages" })).toBeAttached();
  await expect.poll(() => modal.boundingBox()).toEqual(bounds);

  const scroller = modal.getByRole("region", { name: "Messages" });
  await expect(scroller).toHaveAttribute("tabindex", "0");
  await scroller.focus();
  await expect(scroller).toBeFocused();
  await scroller.press("Home");
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBe(0);
  await scroller.press("PageDown");
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);

  // Crossing the inspector breakpoint must preserve the open request and message.
  await page.setViewportSize({ width: 820, height: 900 });
  await expect(modal).toBeVisible();
  await page.keyboard.press("Control+g");
  await expect(page.getByRole("dialog", { name: "Turns", exact: true })).toHaveCount(0);
  await expect(modal.getByRole("button", { name: /^62 user/ })).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(modal).toBeVisible();

  await modal.getByRole("button", { name: /^62 user/ }).click();
  await expect(modal.getByRole("button", { name: /^62 user/ })).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  await expect.poll(() => modal.boundingBox()).toEqual(bounds);
});

test("opens the full recorded turn input in the trace sidebar", async ({ page, baseURL }) => {
  test.setTimeout(90_000);
  await mockSession(page, baseURL!);
  const fullInput = `Prompt of turn 1833\n${"An unabridged line.\n".repeat(500)}End of message.`;
  await page.route(`**/api/v1/platform/sessions/${SESSION}/trace/turns/1833/input`, (route) =>
    route.fulfill({ json: { content: fullInput } }),
  );
  await page.goto(`/sessions/${SESSION}/trace?turn=1833`);
  const turn = page.getByRole("button", { name: /Prompt of turn 1833/ }).locator("..");
  await turn.getByRole("button", { name: "View full message" }).click();
  const sidebar = page.getByRole("complementary", { name: "Turn input" });
  await expect(sidebar).toContainText("End of message.");
  await expect(sidebar.locator("pre")).toHaveText(fullInput);
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("Playground links to a new private Platform Chat", async ({ page, baseURL }) => {
  test.setTimeout(90_000);
  await mockSession(page, baseURL!);
  await page.goto("/playground/new");
  const privateChat = page.getByRole("link", { name: "New private Platform Chat" });
  await expect(privateChat).toHaveAttribute("href", "/chats/new");
  await privateChat.click();
  await expect(page).toHaveURL(/\/chats\/new$/, { timeout: 30_000 });
});

for (const layout of ["phone", "tablet", "beside-chat"] as const) {
  test(`trace keeps a readable timeline and opens details in the ${layout} layout`, async ({
    page,
    baseURL,
  }, info) => {
    await page.setViewportSize(
      layout === "phone"
        ? { width: 390, height: 844 }
        : layout === "tablet"
          ? { width: 820, height: 900 }
          : { width: 1440, height: 900 },
    );
    if (layout === "beside-chat")
      await mockKnowledgeApi(page, { layout: "classic", classicChat: "docked" });
    await mockSession(page, baseURL!, layout === "beside-chat");
    if (layout === "beside-chat")
      await page
        .context()
        .addCookies([
          { name: "everruns_theme", value: "dark", domain: new URL(baseURL!).hostname, path: "/" },
        ]);
    await page.goto(`/sessions/${SESSION}/trace?turn=1833`);
    if (layout === "beside-chat") {
      await page.getByRole("button", { name: "Show chat", exact: true }).click();
      const resize = page.getByRole("separator", { name: "Resize chat", exact: true });
      await resize.focus();
      for (let i = 0; i < 5; i++) await page.keyboard.press("ArrowLeft");
    }
    const trace = page.getByRole("region", { name: "Session trace", exact: true });
    const list = traceList(page);
    await expect(trace.getByRole("button", { name: "Turns", exact: true })).toBeVisible();
    await expect(page.getByRole("list", { name: "Session minimap" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Session details", exact: true })).toBeVisible();
    const box = await list.boundingBox();
    expect(box!.height).toBeGreaterThan(layout === "phone" ? 420 : 490);
    expect(box!.width).toBeGreaterThan(350);
    expect(box!.width).toBeCloseTo((await trace.boundingBox())!.width, 0);
    await trace.getByRole("button", { name: "Filters", exact: true }).click();
    await expect(
      trace.getByRole("checkbox", { name: "Lifecycle events", exact: true }),
    ).toBeVisible();
    await trace.getByRole("button", { name: "Plain text", exact: true }).click();
    await expect(page).toHaveURL(/text=plain/);
    await trace.getByRole("button", { name: "Filters", exact: true }).click();
    await trace.getByRole("button", { name: "Turns", exact: true }).click();
    const turns = page.getByRole("dialog", { name: "Turns", exact: true });
    await turns.getByRole("textbox", { name: "Go to turn", exact: true }).fill("1833");
    await turns.getByRole("textbox", { name: "Go to turn", exact: true }).press("Enter");
    await expect(turns).not.toBeVisible();
    await expect.poll(() => headerInView(page, 1833)).toBe(true);
    const row = list.getByRole("button").filter({ hasText: "#1" }).first();
    await row.click();
    await expect(page).toHaveURL(/step=1833.1/);
    const detail = page.getByRole("dialog", { name: "Step inspector", exact: true });
    await expect(detail).toBeVisible();
    await expect(detail.getByText("Model call", { exact: true })).toBeVisible();
    const detailBox = await detail.boundingBox();
    expect(detailBox!.height).toBeGreaterThan(600);
    if (layout === "phone") {
      await page.reload();
      await expect(detail).toBeVisible();
      await expect(detail.getByText("Model call", { exact: true })).toBeVisible();
    }
    await page.addStyleTag({ content: "nextjs-portal { display: none; }" });
    await info.attach(`${layout} inspector`, {
      body: await detail.screenshot({ path: info.outputPath(`inspector-${layout}@2x.png`) }),
      contentType: "image/png",
    });
    await page.keyboard.press("Escape");
    await expect(detail).not.toBeVisible();
    await expect(page).not.toHaveURL(/step=/);
    await expect.poll(() => headerInView(page, 1833)).toBe(true);
    expect(await page.locator("html").evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(
      true,
    );
    await info.attach(`${layout} trace`, {
      body: await page.screenshot({ path: info.outputPath(`${layout}@2x.png`) }),
      contentType: "image/png",
    });
  });
}
