import { expect, test, type Page } from "@playwright/test";

// The Trace tab's Turns minimap: clicking a cell must bring that range of
// turns into view in the trace, in a session far longer than one loaded page.

const ORG = "org_00000000000000000000000000000001";
const SESSION = "session_00000000000000000000000000000002";
const TURN_COUNT = 1851;
const BUCKET = 16;
test.use({ viewport: { width: 1440, height: 900 } });

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

async function mockSession(page: Page, baseURL: string) {
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
        title: "Long trace fixture",
        tags: [],
        features: [],
        model_id: null,
        status: "idle",
        created_at: "2026-10-01T00:00:00Z",
        updated_at: "2026-10-01T00:00:00Z",
      };
    else if (path === `/api/v1/sessions/${SESSION}/resolved-model`) json = { model_id: null };
    else if (path === `/api/v1/sessions/${SESSION}/trace`)
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
    else if (path === `/api/v1/sessions/${SESSION}/trace/turns`) json = turnsPage(url.searchParams);
    else if (path.startsWith(`/api/v1/sessions/${SESSION}/trace/turns/`)) {
      // turns/{turn}/steps/{step}
      const [turnNo, , stepNo] = path.split("/trace/turns/")[1]!.split("/");
      const detail = turn(Number(turnNo)).items[Number(stepNo) - 1];
      json = path.endsWith("/events")
        ? { events: [] }
        : { step: detail, events: [], input: null, output: null, request: null };
    } else if (path === `/api/v1/orgs/${ORG}`)
      json = { id: ORG, name: "Fixture", default_model_id: null };
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
