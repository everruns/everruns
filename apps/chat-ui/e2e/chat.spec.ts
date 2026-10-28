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
    if (path === "/api/chat/config") {
      await route.continue();
      return;
    }
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
  await page.route("**/api/sessions/*/messages", async (route) => {
    sent = route.request().postDataJSON().message.content[0].text;
    await route.fulfill({ json: { id: "sent" } });
  });
  await page.goto(origin);
  await expect(page.getByText("Migrated files")).toBeVisible();
  await page.getByLabel("Review notes").fill("Ready to share");
  await page.getByLabel("Approve report").check();
  await page.getByRole("button", { name: "Send review", exact: true }).click();
  await expect.poll(() => sent).toContain('"notes":"Ready to share"');
  expect(sent).toContain('"approved":true');
  expect(sent).toContain('"scope":"Entire migration"');
  await expect(page.getByText("255 migration PRs")).toBeVisible();
  await page.getByRole("tab", { name: "Details" }).click();
  await expect(page.getByText("Lit imports: zero")).toBeVisible();
  await page.getByRole("button", { name: "Explain the migration" }).click();
  await expect.poll(() => sent).toBe("Explain the migration");
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
    '```openui\nroot = Card([header, chart])\nheader = CardHeader("Migration activity", "Pull requests per day")\nchart = BarChart(labels, [series])\nlabels = ["Oct 10", "Oct 16"]\nseries = Series("PRs", [11, 92])\n```';
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
  await page.goto(origin);
  await expect(page.getByText("Migration activity", { exact: true })).toBeVisible({
    timeout: 30000,
  });
  await expect(page.locator(".widget svg").first()).toBeVisible();
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
  await page.getByRole("button", { name: "Send ↑", exact: true }).click();
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
