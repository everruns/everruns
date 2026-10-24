import { test, expect } from "@playwright/test";
const origin = "http://01933b5a000070008000000000000001.localhost:9140";
const root = { id: "session_main", title: "Launch research", status: "idle" };
const events = [
  {
    id: "evt_1",
    sequence: 1,
    type: "input.message",
    data: {
      message: {
        id: "message_1",
        role: "user",
        content: [
          {
            type: "text",
            text: "Help me turn our research into a launch plan. Start with what matters most.",
          },
        ],
      },
    },
  },
  {
    id: "evt_2",
    sequence: 2,
    type: "tool.started",
    data: { tool_call: { id: "tool_1", name: "read_file" } },
  },
  {
    id: "evt_3",
    sequence: 3,
    type: "output.message.completed",
    data: {
      message: {
        id: "message_2",
        role: "agent",
        content: [
          {
            type: "text",
            text: "## A focused launch, with room to learn\n\nThe research points to one clear starting point: **make the first successful workflow effortless**. We should lead with the problem people already recognize, then give them a useful result before asking them to configure anything.\n\nI’ve put the working brief in `launch-brief.md`. It covers three priorities:\n\n1. **A clear promise.** Show the outcome in the first screen.\n2. **A small, complete journey.** One task that works from start to finish.\n3. **A feedback loop.** Learn where people pause and what they try next.\n\nThe competitive research is running in a separate thread. When it finishes, I’ll bring back the differences that should change our plan.",
          },
        ],
      },
    },
  },
];
test.beforeEach(async ({ page }) => {
  await page.route("**/api/**", async (route) => {
    const url = new URL(route.request().url()),
      path = url.pathname;
    let data: unknown = {};
    if (path.endsWith("/sse")) {
      await route.fulfill({
        contentType: "text/event-stream",
        body: events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(""),
      });
      return;
    }
    if (path === "/api/sessions")
      data =
        route.request().method() === "POST"
          ? { ...root, id: "session_new", title: "New conversation" }
          : {
              data: [
                root,
                { id: "session_previous", title: "Audience interviews", status: "idle" },
              ],
            };
    else if (path === "/api/chat/main" || path.match(/^\/api\/sessions\/[^/]+$/)) data = root;
    else if (path.endsWith("/work"))
      data = {
        data: [
          {
            id: "task_research",
            title: "Competitive research",
            kind: "assignment",
            state: "running",
            summary: "Comparing onboarding and first-use experiences.",
            links: { child_session_id: "session_research" },
          },
          {
            id: "task_brief",
            title: "Launch brief",
            kind: "assignment",
            state: "succeeded",
            summary: "Ready for review.",
            links: { child_session_id: "session_brief" },
          },
        ],
      };
    else if (path.endsWith("/files"))
      data = {
        data: [
          { name: "launch-brief.md", path: "/workspace/launch-brief.md", is_directory: false },
          { name: "research", path: "/workspace/research", is_directory: true },
        ],
      };
    await route.fulfill({ json: data });
  });
});
test("multi-chat renders transcript, workspace and a new conversation", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  await page.goto(origin);
  await expect(
    page.getByRole("heading", { name: "A focused launch, with room to learn" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Workspace", exact: true }).first().click();
  await expect(page.getByRole("button", { name: "▤ launch-brief.md" })).toBeVisible();
  await page.screenshot({ path: "../../work/work-chat-multi.png", fullPage: true });
  await page.getByRole("button", { name: "New conversation" }).click();
  await expect(page.getByRole("textbox", { name: "Message" })).toBeVisible();
});
test("single chat keeps a main conversation and exposes agent threads", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/single_with_threads");
  await page.goto(origin);
  await expect(page.getByRole("button", { name: "Main conversation" })).toBeVisible();
  await expect(page.getByRole("button", { name: /Competitive research/ })).toBeVisible();
  await expect(page.getByRole("button", { name: "New conversation" })).toHaveCount(0);
  await page.screenshot({ path: "../../work/work-chat-single.png", fullPage: true });
});
test("mobile preserves a usable composer and opens navigation", async ({ page, request }) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(origin);
  await expect(page.getByRole("textbox", { name: "Message" })).toBeVisible();
  await page.getByRole("button", { name: "Toggle conversations" }).click();
  await expect(page.getByRole("button", { name: "New conversation" })).toBeVisible();
  await page.screenshot({ path: "../../work/work-chat-mobile.png", fullPage: true });
});

test("untrusted assistant Markdown cannot execute HTML", async ({ page, request }) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  const unsafe = {
    id: "unsafe",
    sequence: 5,
    type: "output.message.completed",
    data: {
      message: {
        id: "unsafe_message",
        role: "agent",
        content: [
          {
            type: "text",
            text: 'Safe reply\n\n<img src="/missing-image" onerror="window.__chatXss=true">\n<script>window.__chatXss=true</script>\n[unsafe](javascript:window.__chatXss=true)',
          },
        ],
      },
    },
  };
  await page.route("**/api/sessions/*/sse**", (route) =>
    route.fulfill({
      contentType: "text/event-stream",
      body: `data: ${JSON.stringify(unsafe)}\n\n`,
    }),
  );
  await page.goto(origin);
  await expect(page.getByText("Safe reply", { exact: false })).toBeVisible();
  await expect(page.locator(".assistant script")).toHaveCount(0);
  await expect(page.locator(".assistant [onerror]")).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as Window & { __chatXss?: boolean }).__chatXss),
  ).toBeUndefined();
});

test("file editor preserves the revision and explains concurrent changes", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  let revision = "first",
    content = "Original";
  await page.route("**/api/sessions/*/files**", async (route) => {
    const call = route.request(),
      path = new URL(call.url()).searchParams.get("path");
    if (call.method() === "POST") {
      expect(call.postDataJSON().expected_revision).toBe("first");
      revision = "second";
      content = "Agent changed this file";
      await route.fulfill({
        status: 409,
        json: { detail: "File changed. Reload it before saving" },
      });
      return;
    }
    const file = {
      name: "brief.md",
      path: "/workspace/brief.md",
      is_directory: false,
      encoding: "text",
      content,
    };
    await route.fulfill({
      json: path === "/workspace/brief.md" ? { file, revision } : { data: [file] },
    });
  });
  await page.goto(origin);
  await page.getByRole("button", { name: "Workspace", exact: true }).first().click();
  await page.getByRole("button", { name: /brief.md/ }).click();
  await page.getByRole("textbox", { name: "File content" }).fill("My pending edit");
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("alert").filter({ hasText: "File changed." })).toContainText(
    "File changed. Reload it before saving",
  );
  await expect(page.getByRole("textbox", { name: "File content" })).toHaveValue("My pending edit");
  await page.getByRole("button", { name: "Reload file" }).click();
  await expect(page.getByRole("textbox", { name: "File content" })).toHaveValue(
    "Agent changed this file",
  );
});
