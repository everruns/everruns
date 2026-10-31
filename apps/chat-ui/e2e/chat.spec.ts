import { test, expect } from "@playwright/test";
test.use({ deviceScaleFactor: 2 });
const origin = "http://k3m9pq2r7x.localhost:9140";
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
    if (path === "/api/chat/config" || path.startsWith("/api/chat/logo/")) {
      await route.continue();
      return;
    }
    if (path.endsWith("/sse")) {
      await route.fulfill({
        contentType: "text/event-stream",
        body: path.includes("/session_new/")
          ? ""
          : events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(""),
      });
      return;
    }
    if (path === "/api/chat/recovery") data = { scope: "fixture-login-alice" };
    else if (path === "/api/sessions")
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
  const logo = page.locator(".brand-logo");
  await expect(logo).toHaveAttribute(
    "src",
    "/api/chat/logo/avatar_01933b5a000070008000000000000001",
  );
  await expect(logo).toHaveJSProperty("complete", true);
  await expect(logo).not.toHaveJSProperty("naturalWidth", 0);
  expect(
    (
      await request.get(`${origin}/api/chat/logo/avatar_00000000000000000000000000000000`)
    ).status(),
  ).toBe(404);
  await page.getByRole("button", { name: "Workspace", exact: true }).first().click();
  await expect(page.getByRole("button", { name: "▤ launch-brief.md" })).toBeVisible();
  await page.screenshot({ path: "../../work/work-chat-multi.png", fullPage: true });
  await page.getByRole("button", { name: "New conversation" }).click();
  await expect(page.getByRole("textbox", { name: "Message" })).toBeVisible();
  const welcome = page.locator(".mark-logo");
  await expect(welcome).toHaveAttribute(
    "src",
    "/api/chat/logo/avatar_01933b5a000070008000000000000001",
  );
  await expect(welcome).toHaveJSProperty("complete", true);
  await expect(welcome).not.toHaveJSProperty("naturalWidth", 0);
});
test("single chat keeps a main conversation and exposes agent threads", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/single_with_threads");
  await page.goto(origin);
  await expect(page.getByRole("button", { name: "Main conversation" })).toBeVisible();
  await expect(
    page
      .getByRole("navigation", { name: "Conversations" })
      .getByRole("button", { name: /Competitive research/ }),
  ).toBeVisible();
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

test("a sent message is the reply, not a work-log preview", async ({ page, request }) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  const events = [
    {
      id: "evt_user",
      sequence: 1,
      type: "input.message",
      data: {
        message: {
          id: "message_user",
          role: "user",
          content: [{ type: "text", text: "What do we see here?" }],
        },
      },
    },
    {
      id: "evt_send",
      sequence: 2,
      type: "tool.started",
      data: {
        tool_call: { id: "call_send", name: "send_message" },
        display_name: "Send message",
        narration:
          "Sending message: It's the GitHub **About** panel for **Bashkit** (`bashkit.sh`).",
      },
    },
    {
      id: "evt_reply",
      sequence: 3,
      type: "conversation.message",
      data: {
        message_id: "message_reply",
        text: "It's the GitHub **About** panel for **Bashkit** (`bashkit.sh`), described as a shell toolkit.",
      },
    },
    {
      id: "evt_done",
      sequence: 4,
      type: "tool.completed",
      data: { tool_call_id: "call_send", tool_name: "send_message", display_name: "Send message" },
    },
    { id: "evt_turn", sequence: 5, type: "turn.completed", data: { turn_id: "turn_1" } },
  ];
  await page.route("**/api/sessions/*/sse**", (route) =>
    route.fulfill({
      contentType: "text/event-stream",
      body: events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(""),
    }),
  );
  await page.goto(origin);
  const reply = page.locator(".message.assistant");
  await expect(reply).toContainText("It's the GitHub About panel for Bashkit");
  await expect(reply).toContainText("bashkit.sh");
  await expect(reply).not.toContainText("**");
  await expect(reply).not.toContainText("Sending message");
  await expect(page.locator(".activity")).toHaveCount(0);
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
  await page.getByRole("button", { name: "Edit source" }).click();
  await page.getByRole("textbox", { name: "File content" }).fill("My pending edit");
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("alert").filter({ hasText: "File changed." })).toContainText(
    "File changed. Reload it before saving",
  );
  await expect(page.getByRole("textbox", { name: "File content" })).toHaveValue("My pending edit");
  await page.getByRole("button", { name: "Reload file" }).click();
  await expect(page.getByText("Agent changed this file", { exact: true })).toBeVisible();
});

test("renders inline interactive widgets and sends their message actions", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  const widget = {
    type: "Card",
    props: { title: "Migration progress" },
    children: [
      {
        type: "List",
        children: [{ type: "ListItem", props: { title: "Migrated files", value: "796" } }],
      },
      {
        type: "Form",
        props: { name: "Review", submitLabel: "Send review" },
        children: [
          { type: "Textarea", props: { name: "notes", label: "Review notes" } },
          { type: "Checkbox", props: { name: "approved", label: "Approve report" } },
          {
            type: "TextField",
            props: { name: "scope", label: "Review scope", defaultValue: "Entire migration" },
          },
        ],
      },
      {
        type: "Tabs",
        children: [
          {
            type: "Tab",
            props: { label: "Summary" },
            children: [{ type: "Text", props: { text: "255 migration PRs" } }],
          },
          {
            type: "Tab",
            props: { label: "Details" },
            children: [{ type: "Text", props: { text: "Lit imports: zero" } }],
          },
        ],
      },
      {
        type: "Button",
        props: {
          label: "Explain the migration",
          action: { type: "message", text: "Explain the migration" },
        },
      },
    ],
  };
  await page.route("**/api/sessions/*/sse**", (route) =>
    route.fulfill({
      contentType: "text/event-stream",
      body: `data: ${JSON.stringify({ id: "widget", sequence: 4, type: "output.message.completed", data: { message: { id: "widget", role: "agent", content: [{ type: "text", text: "```a2ui\n" + JSON.stringify(widget) + "\n```" }] } } })}\n\n`,
    }),
  );
  let sent = "";
  let sentFiles = -1;
  let attempts = 0;
  let firstKey = "";
  await page.route("**/api/sessions/*/messages", async (route) => {
    const body = route.request().postDataJSON();
    sent = body.message.content[0].text;
    sentFiles = body.files.length;
    attempts++;
    if (attempts === 1) {
      firstKey = route.request().headers()["idempotency-key"];
      await route.abort("failed");
    } else {
      if (attempts === 2) expect(route.request().headers()["idempotency-key"]).toBe(firstKey);
      await route.fulfill({ json: { id: "sent" } });
    }
  });
  await page.goto(origin);
  await expect(page.getByText("Migrated files")).toBeVisible();
  await page.getByRole("textbox", { name: "Message", exact: true }).fill("Unsent note");
  await page.getByLabel("Attach files").setInputFiles({
    name: "private-draft.csv",
    mimeType: "text/csv",
    buffer: Buffer.from("pending file"),
  });
  await page.getByLabel("Review notes").fill("Ready to share");
  await page.getByLabel("Approve report").check();
  await page.getByRole("button", { name: "Send review", exact: true }).click();
  await expect.poll(() => sent).toContain('"notes":"Ready to share"');
  expect(sent).toContain('"approved":true');
  expect(sent).toContain('"scope":"Entire migration"');
  expect(sentFiles).toBe(0);
  await page.getByRole("button", { name: "Retry message", exact: true }).click();
  await expect(page.getByRole("button", { name: "Retry message", exact: true })).toHaveCount(0);
  await expect(page.getByRole("textbox", { name: "Message", exact: true })).toHaveValue(
    "Unsent note",
  );
  await expect(page.getByRole("button", { name: "Remove private-draft.csv" })).toBeVisible();
  await expect(page.getByText("255 migration PRs")).toBeVisible();
  await page.getByRole("tab", { name: "Details" }).click();
  await expect(page.getByText("Lit imports: zero")).toBeVisible();
  await page.getByRole("button", { name: "Explain the migration" }).click();
  await expect.poll(() => sent).toBe("Explain the migration");
  expect(sentFiles).toBe(0);
  await expect(page.getByRole("textbox", { name: "Message", exact: true })).toHaveValue(
    "Unsent note",
  );
  await expect(page.getByRole("button", { name: "Remove private-draft.csv" })).toBeVisible();
});

test("previews and filters CSV data and downloads text files", async ({ page, request }) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  await page.route("**/api/sessions/*/files**", (route) =>
    route.fulfill({
      json:
        new URL(route.request().url()).searchParams.get("path") === "/workspace/timeline.csv"
          ? {
              file: {
                name: "timeline.csv",
                path: "/workspace/timeline.csv",
                encoding: "text",
                content: 'date,description\nOct 10,"First PR, opened"\nOct 16,Lit removed',
              },
              revision: "first",
            }
          : {
              data: [
                { name: "timeline.csv", path: "/workspace/timeline.csv", is_directory: false },
              ],
            },
    }),
  );
  await page.goto(origin);
  await page.getByRole("button", { name: "Workspace", exact: true }).first().click();
  await page.getByRole("button", { name: /▤ timeline.csv/ }).click();
  await expect(page.getByRole("cell", { name: "First PR, opened" })).toBeVisible();
  await page.getByRole("textbox", { name: "Filter table" }).fill("Lit");
  await expect(page.getByRole("cell", { name: "First PR, opened" })).toHaveCount(0);
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Download", exact: true }).click();
  expect((await download).suggestedFilename()).toBe("timeline.csv");
});

test("published conversations have widgets but no composer or workspace access", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  await page.route("**/api/chat/shared/**", (route) =>
    route.fulfill({ json: { title: "Shared migration report", data: events, before: null } }),
  );
  await page.goto(origin + "?share=public-token");
  await expect(page.getByText("Public · Read-only")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "A focused launch, with room to learn" }),
  ).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Message" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Workspace", exact: true })).toHaveCount(0);
});

test("HTML artifacts cannot execute scripts or fetch remote resources", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  let external = false;
  await page.route("**/evil.example/**", (route) => {
    external = true;
    return route.abort();
  });
  await page.route("**/api/sessions/*/files**", (route) =>
    route.fulfill({
      json:
        new URL(route.request().url()).searchParams.get("path") === "/workspace/report.html"
          ? {
              file: {
                name: "report.html",
                path: "/workspace/report.html",
                encoding: "text",
                content:
                  '<h1>Report preview</h1><script>parent.__previewXss=true</script><img src="https://evil.example/leak">',
              },
              revision: "first",
            }
          : {
              data: [{ name: "report.html", path: "/workspace/report.html", is_directory: false }],
            },
    }),
  );
  await page.goto(origin);
  await page.getByRole("button", { name: "Workspace", exact: true }).first().click();
  await page.getByRole("button", { name: /▤ report.html/ }).click();
  await expect(
    page
      .frameLocator('iframe[title="Preview report.html"]')
      .getByRole("heading", { name: "Report preview" }),
  ).toBeVisible();
  expect(await page.locator('iframe[title="Preview report.html"]').getAttribute("sandbox")).toBe(
    "",
  );
  expect(
    await page.evaluate(() => (window as Window & { __previewXss?: boolean }).__previewXss),
  ).toBeUndefined();
  expect(external).toBe(false);
});

test("renders an OpenUI report with a chart instead of source code", async ({ page, request }) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  const text =
    '```openui\nroot = Card([header, chart, action])\naction = Button("Explain this chart")\nheader = CardHeader("Migration activity", "Pull requests per day")\nchart = BarChart(labels, [series])\nlabels = ["Oct 10", "Oct 16"]\nseries = Series("PRs", [11, 92])\n```';
  await page.route("**/api/sessions/*/sse**", (route) =>
    route.fulfill({
      contentType: "text/event-stream",
      body: `data: ${JSON.stringify({ id: "chart", sequence: 4, type: "output.message.completed", data: { message: { id: "chart", role: "agent", content: [{ type: "text", text }] } } })}\n\n`,
    }),
  );
  await page.route("**/api/sessions**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === "/api/sessions")
      return route.fulfill({
        json: {
          data: [
            { ...root, title: "UI migration timeline" },
            { ...root, id: "session_notes", title: "Release notes" },
          ],
        },
      });
    if (path === "/api/sessions/session_main")
      return route.fulfill({ json: { ...root, title: "UI migration timeline" } });
    return route.fallback();
  });
  let chartAction = "";
  await page.route("**/api/sessions/*/messages", (route) => {
    chartAction = route.request().postDataJSON().message.content[0].text;
    return route.fulfill({ json: { id: "chart-action" } });
  });
  await page.goto(origin);
  await expect(page.getByText("Migration activity", { exact: true })).toBeVisible({
    timeout: 30000,
  });
  await expect(page.locator(".widget svg").first()).toBeVisible();
  await page.getByRole("button", { name: "Explain this chart", exact: true }).click();
  await expect.poll(() => chartAction).toBe("Explain this chart");
  await expect(page.getByText("root = Card", { exact: false })).toHaveCount(0);
  await page.screenshot({ path: "../../work/work-chat-report.png", fullPage: true });
});

test("preserves attachments and draft on failure, then submits effort and all files", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  let attempts = 0;
  let sent:
    | { files: Array<{ name: string; data: string }>; controls: { reasoning: { effort: string } } }
    | undefined;
  await page.route("**/api/sessions/*/messages", async (route) => {
    attempts++;
    sent = route.request().postDataJSON();
    await route.fulfill(
      attempts === 1
        ? { status: 503, json: { error: "Please retry" } }
        : { status: 201, json: { id: "sent" } },
    );
  });
  await page.goto(origin);
  await page.getByRole("textbox", { name: "Message", exact: true }).fill("Compare these reports");
  await page.getByLabel("Attach files").setInputFiles([
    { name: "before.csv", mimeType: "text/csv", buffer: Buffer.from("count\n796") },
    { name: "after.csv", mimeType: "text/csv", buffer: Buffer.from("count\n0") },
  ]);
  await page.getByRole("combobox", { name: "Reasoning effort" }).selectOption("high");
  await page.getByRole("button", { name: "Send ↑", exact: true }).click();
  await expect(page.getByRole("alert").filter({ hasText: "Please retry" })).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Message", exact: true })).toHaveValue(
    "Compare these reports",
  );
  await expect(page.getByRole("button", { name: "Remove before.csv" })).toBeVisible();
  await page.getByRole("button", { name: "Retry message", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "Message", exact: true })).toHaveValue("");
  expect(sent?.files.map((file) => file.name)).toEqual(["before.csv", "after.csv"]);
  expect(Buffer.from(sent!.files[0].data, "base64").toString()).toBe("count\n796");
  expect(sent?.controls.reasoning.effort).toBe("high");
  await expect(page.getByRole("button", { name: "Remove before.csv" })).toHaveCount(0);
});

test("publishes a usable read-only link when clipboard access is denied and can revoke it", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  const token = "01933b5a-0000-7000-8000-000000000001";
  let revoked = false;
  await page.route("**/api/sessions/*/share", async (route) => {
    revoked = route.request().method() === "DELETE";
    await route.fulfill({ json: revoked ? { revoked: true } : { token } });
  });
  await page.addInitScript(() =>
    Object.defineProperty(navigator, "clipboard", {
      value: {
        writeText: async () => {
          throw new Error("Clipboard access denied");
        },
      },
    }),
  );
  page.on("dialog", (dialog) => dialog.accept());
  await page.goto(origin);
  await page.getByLabel("Conversation actions").click();
  await page.getByRole("button", { name: "Share read-only link", exact: true }).click();
  await expect(
    page.getByText(`Read-only link: ${origin}/?share=${token}`, { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(`Read-only link: ${origin}/?share=${token}`, { exact: true }),
  ).not.toContainText(root.id);
  await page.getByRole("button", { name: "Revoke public links", exact: true }).click();
  await expect(page.getByText("Public links revoked.")).toBeVisible();
  expect(revoked).toBe(true);
});

test("draft and uncertain message survive reload; Retry preserves its idempotency key", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  const attempts: Array<{ key?: string; body: string | null; url: string }> = [];
  await page.route("**/api/sessions/*/messages", async (route) => {
    attempts.push({
      key: route.request().headers()["idempotency-key"],
      body: route.request().postData(),
      url: route.request().url(),
    });
    if (attempts.length === 1) await route.abort("failed");
    else await route.fulfill({ json: { id: "message_accepted" } });
  });
  await page.goto(origin);
  const composer = page.getByRole("textbox", { name: "Message", exact: true });
  await expect(composer).toBeEnabled();
  await composer.fill("Keep this instruction through a connection loss.");
  await page.reload();
  await expect(composer).toHaveValue("Keep this instruction through a connection loss.");
  await composer.press("Enter");
  await expect(page.getByRole("button", { name: "Retry message" })).toBeEnabled();
  await page.reload();
  await expect(page.getByRole("button", { name: "Retry message" })).toBeEnabled();
  expect(attempts).toHaveLength(1);
  if (process.env.CHAT_RECOVERY_SCREENSHOT)
    await page.locator(".composer-area").screenshot({ path: process.env.CHAT_RECOVERY_SCREENSHOT });
  await composer.fill("A newer instruction waiting in the composer.");
  await page.getByRole("button", { name: "Retry message" }).click();
  await expect(page.getByRole("button", { name: "Retry message" })).toHaveCount(0);
  expect(attempts).toHaveLength(2);
  expect(attempts[1]).toEqual(attempts[0]);
  await expect(composer).toHaveValue("A newer instruction waiting in the composer.");
});

test("switching conversations isolates drafts and rejects duplicate Enter while sending", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  let requests = 0;
  await page.route("**/api/sessions/*/messages", async (route) => {
    requests++;
    await new Promise((resolve) => setTimeout(resolve, 300));
    await route.fulfill({ json: { id: "accepted" } });
  });
  await page.goto(origin);
  const composer = page.getByRole("textbox", { name: "Message", exact: true });
  await expect(composer).toBeEnabled();
  await composer.fill("First conversation draft");
  await page.getByRole("button", { name: "Audience interviews" }).click();
  await expect(composer).toHaveValue("");
  await composer.fill("Second conversation message");
  await composer.press("Enter");
  await composer.press("Enter");
  await expect(composer).toHaveValue("");
  expect(requests).toBe(1);
  await page.getByRole("button", { name: "Launch research" }).click();
  await expect(composer).toHaveValue("First conversation draft");
});

test("an authorization rejection stops stream reconnection", async ({ page, request }) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  let streams = 0;
  await page.route("**/api/sessions/*/sse**", async (route) => {
    streams++;
    await route.fulfill({ status: 401, body: "Sign in to continue" });
  });
  await page.goto(origin);
  await expect(page.getByRole("link", { name: "Continue to sign in →" })).toBeVisible({
    timeout: 15000,
  });
  await expect(page.getByText("Sign in to continue")).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Sign in", exact: true })).toHaveCount(1);
  await page.waitForTimeout(2200);
  expect(streams).toBe(1);
  await expect(page.getByText("Sign in to continue")).toHaveCount(0);
});

test("the sidebar shows the signed-in person", async ({ page, request }) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  await page.route("**/api/chat/profile", async (route) => {
    await route.fulfill({
      json: {
        name: "Ada Lovelace",
        email: "ada@example.com",
        picture: "javascript:alert(1)",
      },
    });
  });
  await page.goto(origin);
  const account = page.locator("aside footer .account");
  await expect(account.getByText("Ada Lovelace")).toBeVisible();
  await expect(account.getByText("ada@example.com")).toBeVisible();
  await expect(account.locator(".avatar")).toHaveText("AL");
  await expect(account.locator("img")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Sign out", exact: true })).toBeVisible();
  await expect(page.locator("aside footer").getByText("Work Chat", { exact: true })).toHaveCount(0);
});

test("sign-out retires an attachment read before it can start a message", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  await page.addInitScript(() => {
    const actualRead = FileReader.prototype.readAsDataURL;
    FileReader.prototype.readAsDataURL = function (file) {
      const originalLoad = this.onload;
      this.onload = (event) => {
        originalLoad?.call(this, event);
        (window as unknown as { uploadReadFinished: boolean }).uploadReadFinished = true;
      };
      (window as unknown as { releaseUpload: () => void }).releaseUpload = () =>
        actualRead.call(this, file);
    };
  });
  let finishLogout!: () => void;
  const logout = new Promise<void>((resolve) => {
    finishLogout = resolve;
  });
  await page.route("**/auth/logout", async (route) => {
    await logout;
    await route.fulfill({ status: 204 });
  });
  let messages = 0;
  await page.route("**/api/sessions/*/messages", async (route) => {
    messages++;
    await route.fulfill({ json: { id: "unexpected" } });
  });
  await page.goto(origin);
  const composer = page.getByRole("textbox", { name: "Message", exact: true });
  await expect(composer).toBeEnabled();
  await page
    .locator('input[type="file"]')
    .setInputFiles({ name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("notes") });
  await composer.fill("Read the attachment.");
  await composer.press("Enter");
  await page.waitForFunction(
    () => typeof (window as unknown as { releaseUpload?: () => void }).releaseUpload === "function",
  );
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await page.evaluate(() => (window as unknown as { releaseUpload: () => void }).releaseUpload());
  await page.waitForFunction(
    () => (window as unknown as { uploadReadFinished?: boolean }).uploadReadFinished,
  );
  await page.waitForTimeout(200);
  const saved = await page.evaluate(() =>
    Object.keys(sessionStorage).filter((key) => key.startsWith("everruns:work-chat:")),
  );
  const observedMessages = messages;
  finishLogout();
  expect(observedMessages).toBe(0);
  expect(saved).toEqual([]);
});

test("A2UI 1.0 renders bound forms, charts, tabs and updates in both themes", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  const operations = [
    {
      version: "v1.0",
      createSurface: {
        surfaceId: "review",
        catalogId: "copilotkit/standard@1",
        dataModel: { name: "", total: 12 },
        components: [
          {
            id: "root",
            component: "Column",
            children: ["heading", "tabs", "form", "chart", "html"],
          },
          { id: "heading", component: "Title", text: "Release review" },
          {
            id: "tabs",
            component: "Tabs",
            tabs: [
              { title: "Overview", child: "overview" },
              { title: "Details", child: "details" },
            ],
          },
          {
            id: "overview",
            component: "Text",
            text: { "@call": "formatString", args: { value: "Reviewed ${/total} items" } },
          },
          { id: "details", component: "Text", text: "All checks complete" },
          {
            id: "form",
            component: "Form",
            label: "Release review",
            children: ["name", "submit"],
            onSubmit: {
              event: {
                name: "review",
                userMessage: "Submit review",
                context: { name: { "@path": "/name" } },
              },
            },
          },
          { id: "name", component: "TextField", label: "Reviewer", value: { "@path": "/name" } },
          {
            id: "submit",
            component: "Button",
            label: "Send review",
            submit: true,
            checks: [
              {
                condition: { "@call": "required", args: { value: { "@path": "/name" } } },
                message: "Reviewer is required",
              },
            ],
          },
          {
            id: "chart",
            component: "Chart",
            data: [
              { week: "Week 1", items: 5 },
              { week: "Week 2", items: 12 },
            ],
            xKey: "week",
            series: ["items"],
            chartType: "bar",
          },
          {
            id: "html",
            component: "HtmlView",
            height: 100,
            title: "Safe preview",
            html: '<meta http-equiv="refresh" content="60;url=/widget-navigation-leak"><p>Sandboxed preview</p><a href="/widget-navigation-leak">External destination</a><script>parent.__widgetScript = true</script>',
          },
        ],
      },
    },
  ];
  const text =
    "A visual release review.\n```a2ui\n" +
    operations.map((o) => JSON.stringify(o)).join("\n") +
    "\n```\n```a2ui\n" +
    JSON.stringify({
      version: "v1.0",
      updateDataModel: { surfaceId: "review", path: "/total", value: 25 },
    }) +
    "\n```";
  await page.route("**/api/sessions/*/sse**", (route) =>
    route.fulfill({
      contentType: "text/event-stream",
      body: `data: ${JSON.stringify({ id: "a2ui", sequence: 4, type: "output.message.completed", data: { message: { id: "a2ui", role: "agent", content: [{ type: "text", text }] } } })}\n\n`,
    }),
  );
  let sent = "";
  await page.route("**/api/sessions/*/messages", (route) => {
    sent = route.request().postDataJSON().message.content[0].text;
    return route.fulfill({ json: { id: "sent" } });
  });
  await page.goto(origin);
  await expect(page.getByRole("heading", { name: "Release review" })).toBeVisible({
    timeout: 30000,
  });
  await expect(page.getByText("Reviewed 25 items")).toBeVisible();
  await expect(page.locator(".everruns-widget .recharts-surface").first()).toBeVisible();
  await page.getByRole("tab", { name: "Details", exact: true }).click();
  await expect(page.getByText("All checks complete")).toBeVisible();
  await expect(page.getByRole("button", { name: "Send review", exact: true })).toBeDisabled();
  await page.getByRole("textbox", { name: "Reviewer", exact: true }).fill("Alex");
  await expect(page.getByRole("button", { name: "Send review", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Send review", exact: true }).click();
  await expect.poll(() => sent).toContain("Submit review");
  expect(sent).toContain('"name":"Alex"');
  expect(
    await page.evaluate(() => (window as unknown as { __widgetScript?: boolean }).__widgetScript),
  ).toBeUndefined();
  await expect(page.locator('iframe[title="Safe preview"]')).toHaveAttribute("sandbox", "");
  expect(await page.locator('iframe[title="Safe preview"]').getAttribute("srcdoc")).not.toContain(
    "widget-navigation-leak",
  );
  await page.setViewportSize({ width: 1440, height: 1500 });
  await page.mouse.move(0, 0);
  await page.screenshot({ path: "/tmp/everruns-a2ui-light.png", fullPage: true });
  await page
    .locator(".message.assistant")
    .first()
    .screenshot({ path: "/tmp/everruns-a2ui-widget@2x.png" });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await expect(page.getByRole("heading", { name: "Release review" })).toBeVisible();
  await page.screenshot({ path: "/tmp/everruns-a2ui-dark.png", fullPage: true });
});

test("all 69 standard components render from the catalog fixtures", async ({ page, request }) => {
  let mapStyleRequested = false;
  await page.route("https://tiles.openfreemap.org/**", (route) => {
    mapStyleRequested = true;
    return route.fulfill({ json: { version: 8, sources: {}, layers: [] } });
  });
  const { default: samples } = await import("../../ui/generative-ui/tests/standard-samples.json", {
    with: { type: "json" },
  });
  expect(new Set(samples.flatMap((sample) => sample.covers)).size).toBe(69);
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  const text = samples
    .map(
      (sample, index) =>
        "```a2ui\n" +
        JSON.stringify({
          version: "v1.0",
          createSurface: {
            surfaceId: `sample-${index}`,
            catalogId: "copilotkit/standard@1",
            components: sample.components,
            dataModel: sample.data,
          },
        }) +
        "\n```",
    )
    .join("\n");
  await page.route("**/api/sessions/*/sse**", (route) =>
    route.fulfill({
      contentType: "text/event-stream",
      body: `data: ${JSON.stringify({ id: "catalog", sequence: 4, type: "output.message.completed", data: { message: { id: "catalog", role: "agent", content: [{ type: "text", text }] } } })}\n\n`,
    }),
  );
  const failures: string[] = [];
  page.on("console", (message) => {
    if (message.text().includes("[generativeUI]")) failures.push(message.text());
  });
  page.on("pageerror", (error) => failures.push(error.message));
  await page.goto(origin);
  await expect(page.locator("[data-generative-ui-root]")).toHaveCount(samples.length, {
    timeout: 30000,
  });
  await expect(page.getByText("Part of this UI couldn't be shown.", { exact: true })).toHaveCount(
    0,
  );
  await expect.poll(() => mapStyleRequested).toBe(true);
  expect(failures).toEqual([]);
});

test("rejected A2UI surfaces show source and do not break later replies", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  const text =
    '```a2ui\n{"version":"v1.0","createSurface":{"surfaceId":"bad","catalogId":"copilotkit/standard@1","components":[{"id":"root","component":"UnregisteredWidget"}]}}\n```\nA readable explanation still follows.';
  await page.route("**/api/sessions/*/sse**", (route) =>
    route.fulfill({
      contentType: "text/event-stream",
      body: `data: ${JSON.stringify({ id: "bad", sequence: 4, type: "output.message.completed", data: { message: { id: "bad", role: "agent", content: [{ type: "text", text }] } } })}\n\n`,
    }),
  );
  await page.goto(origin);
  await expect(
    page.getByText("This content could not be displayed.", { exact: true }),
  ).toBeVisible();
  await page.getByText("This content could not be displayed.", { exact: true }).click();
  await expect(page.getByText(/UnregisteredWidget/)).toBeVisible();
  await expect(
    page.getByText("A readable explanation still follows.", { exact: true }),
  ).toBeVisible();
});

test("public A2UI 1.0 shares render with execution controls disabled", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:9141/mode/multi_chat");
  const text =
    '```a2ui\n{"version":"v1.0","createSurface":{"surfaceId":"shared","catalogId":"copilotkit/standard@1","components":[{"id":"root","component":"Button","label":"Send private action","onClick":{"event":{"name":"private","userMessage":"Run private action"}}}]}}\n```';
  await page.route("**/api/chat/shared/**", (route) =>
    route.fulfill({
      json: {
        title: "Shared review",
        data: [
          {
            id: "shared",
            sequence: 1,
            type: "output.message.completed",
            data: { message: { id: "shared", role: "agent", content: [{ type: "text", text }] } },
          },
        ],
        before: null,
      },
    }),
  );
  let sent = 0;
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().includes("/messages")) sent++;
  });
  await page.goto(origin + "?share=public-token");
  await expect(
    page.getByRole("button", { name: "Send private action", exact: true }),
  ).toBeDisabled();
  expect(sent).toBe(0);
});
