import { test, expect } from "@playwright/test";
test.use({ deviceScaleFactor: 2 });
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
  await page.waitForTimeout(2200);
  expect(streams).toBe(1);
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
