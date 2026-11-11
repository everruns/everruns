import { expect, test } from "@playwright/test";
import { DOCS_ID, PLAYBOOKS_ID, mockKnowledgeApi } from "./fixtures/knowledge-app";

for (const role of ["owner", "admin", "member"] as const) {
  test(`new Tracing UI respects ${role} access beside Chat`, async ({ context, page, baseURL }) => {
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
    ]);
    await mockKnowledgeApi(page, { role });
    await page.route("**/api/v1/tracing/**", async (route) => {
      const path = new URL(route.request().url()).pathname;
      const json = path.endsWith("/defaults")
        ? { revision: 1, policy: { mode: "replace", destination_ids: [] } }
        : [
            {
              id: "00000000-0000-4000-8000-000000000001",
              revision: 1,
              config: {
                name: "Fictional receiver",
                provider: "otlp",
                endpoint: "https://example.test",
                project_id: null,
                capture_content: false,
                capture_reasoning: false,
                enabled: false,
              },
              last_error: null,
              last_attempt_at: null,
              last_accepted_at: null,
              dropped_events: 0,
              organization_default: false,
              agent_selections: 0,
            },
          ];
      await route.fulfill({ json });
    });
    await page.goto("/settings/tracing");
    await expect(page.getByRole("cell", { name: /^Fictional receiver/ })).toBeVisible();
    await expect(page.locator(".chat-mode-shell")).toHaveAttribute("data-layout", "beside");
    await expect(page.getByRole("region", { name: "Current page" })).toBeVisible();
    await page
      .getByRole("button", {
        name: role === "member" ? "View defaults" : "Edit defaults",
        exact: true,
      })
      .click();
    const selection = page.getByRole("checkbox");
    if (role === "member") {
      await expect(selection).toBeDisabled();
      await expect(page.getByRole("button", { name: "Add destination", exact: true })).toHaveCount(
        0,
      );
    } else {
      await selection.check();
      await expect(page.getByRole("button", { name: "Save changes", exact: true })).toBeEnabled();
      await page.getByRole("button", { name: "Add destination", exact: true }).click();
      await expect(page.getByRole("dialog")).toBeVisible();
      await page.getByRole("button", { name: "Cancel", exact: true }).click();
    }
  });
}

// Rendered browser coverage uses fictional API fixtures. Real LLM protocol,
// authority and revision checks have separate integration/controller coverage.
test("Knowledge UI beside Main chat keeps page drafts", async ({ context, page, baseURL }) => {
  // CI uses Next's development server; these tabs can require a cold compile.
  test.setTimeout(60_000);
  await context.addCookies([
    { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
  ]);
  await mockKnowledgeApi(page);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(`/knowledge/${PLAYBOOKS_ID}?file=%2Fvip-escalation.md`);
  await expect(page.getByRole("heading", { name: "support-playbooks", exact: true })).toBeVisible();
  await expect(page.getByText("Budgets cap what a Session may spend.")).toBeVisible();
  // Chat first is the default layout: Main chat already sits beside the page.
  await expect(page.locator(".chat-mode-shell")).toHaveAttribute("data-layout", "beside");
  await expect(page.getByRole("region", { name: "Current page" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Platform Chat", exact: true })).toBeVisible();
  await expect(page.getByRole("separator", { name: "Resize chat" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Thread not found" })).toHaveCount(0);
  const nav = page.getByRole("navigation", { name: "Collection sections" });
  await nav.getByRole("link", { name: "Sources" }).click();
  await page.waitForURL(`**/knowledge/${PLAYBOOKS_ID}/sources`);
  await expect(page.getByRole("cell", { name: /Managed files/ })).toBeVisible();
  await nav.getByRole("link", { name: "Settings" }).click();
  await page.waitForURL(`**/knowledge/${PLAYBOOKS_ID}/settings`);
  const name = page.getByRole("textbox", { name: "Name", exact: true });
  await expect(name).toHaveValue("support-playbooks");
  await name.fill("Support draft");
  // Cmd/Ctrl-J focuses the composer; the page and its draft stay put.
  await page.keyboard.press("Control+j");
  await expect(page.locator(".chat-mode-shell")).toHaveAttribute("data-layout", "beside");
  await expect(name).toHaveValue("Support draft");
  await page.getByRole("separator", { name: "Resize chat" }).dblclick();
  await expect(name).toHaveValue("Support draft");
  expect(errors).toEqual([]);
});

for (const role of ["owner", "admin", "member"] as const) {
  test(`new Knowledge settings and source controls respect ${role}`, async ({
    context,
    page,
    baseURL,
  }) => {
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
    ]);
    await mockKnowledgeApi(page, { role });
    await page.goto(`/knowledge/${PLAYBOOKS_ID}/settings`);
    if (role === "member") {
      await expect(
        page.getByText("Only organization owners and admins can change a collection."),
      ).toBeVisible();
      await expect(page.getByRole("textbox", { name: "Name", exact: true })).toHaveCount(0);
    } else {
      await expect(page.getByRole("textbox", { name: "Name", exact: true })).toHaveValue(
        "support-playbooks",
      );
    }
    await page.goto(`/knowledge/${PLAYBOOKS_ID}/sources`);
    await expect(page.getByRole("cell", { name: /Managed files/ })).toBeVisible();
    await expect(page.getByRole("button", { name: "Connect a source" })).toHaveCount(
      role === "member" ? 0 : 1,
    );
    await page.goto(`/knowledge/${DOCS_ID}/settings`);
    if (role !== "member")
      await expect(page.getByRole("textbox", { name: "Name", exact: true })).toBeDisabled();
  });
}

for (const theme of ["light", "dark"] as const) {
  test(`Knowledge browser uses page-pane width in ${theme} mode`, async ({
    context,
    page,
    baseURL,
  }, testInfo) => {
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
      { name: "everruns_theme", value: theme, domain: new URL(baseURL!).hostname, path: "/" },
    ]);
    await mockKnowledgeApi(page);
    await page.setViewportSize({ width: 1200, height: 1000 });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto(`/knowledge/${PLAYBOOKS_ID}?file=%2Fvip-escalation.md`);
    await expect(page.getByText("Budgets cap what a Session may spend.")).toBeVisible();
    await expect(page.locator(".chat-mode-shell")).toHaveAttribute("data-layout", "beside");
    const tree = await page.getByRole("tree").boundingBox();
    const content = await page.getByText("Budgets cap what a Session may spend.").boundingBox();
    expect(tree).not.toBeNull();
    expect(content).not.toBeNull();
    expect(content!.y).toBeGreaterThan(tree!.y + tree!.height);
    const region = page.getByRole("region", { name: "Current page" });
    expect(await region.evaluate((node) => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
    await testInfo.attach("Knowledge split workspace", {
      body: await page.screenshot({
        path: testInfo.outputPath("knowledge-pane.png"),
        fullPage: true,
      }),
      contentType: "image/png",
    });
    // Narrow screens fall back to the classic, single-pane layout.
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(page.locator(".chat-mode-shell")).toHaveAttribute("data-layout", "classic");
    await expect(
      page.getByRole("heading", { name: "support-playbooks", exact: true }),
    ).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(
      true,
    );
    await testInfo.attach("Knowledge mobile", {
      body: await page.screenshot({
        path: testInfo.outputPath("knowledge-mobile.png"),
        fullPage: true,
      }),
      contentType: "image/png",
    });
  });
}

// Chat first: the URL alone describes the screen, so browser Back and Forward
// step through pages, full-screen chat and threads opened beside the chat.
test("browser back and forward step through pages, full screen and threads", async ({
  context,
  page,
  baseURL,
}) => {
  test.setTimeout(90_000);
  await context.addCookies([
    { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
  ]);
  await mockKnowledgeApi(page);
  const threadId = "session_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
  await page.route(`**/api/v1/sessions/${threadId}`, (route) =>
    route.fulfill({
      json: {
        id: threadId,
        organization_id: "org_00000000000000000000000000000001",
        agent_id: "agent_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        harness_id: "harness_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        status: "idle",
        source: "chat",
        tags: [],
        title: "Fictional release notes",
        parent_session_id: "session_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        created_at: "2026-10-27T10:00:00Z",
        updated_at: "2026-10-27T10:00:00Z",
      },
    }),
  );
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(`/knowledge/${PLAYBOOKS_ID}`);
  const shell = page.locator(".chat-mode-shell");
  await expect(shell).toHaveAttribute("data-layout", "beside");
  const nav = page.getByRole("navigation", { name: "Collection sections" });
  await nav.getByRole("link", { name: "Sources" }).click();
  await page.waitForURL(`**/knowledge/${PLAYBOOKS_ID}/sources`);
  await nav.getByRole("link", { name: "Settings" }).click();
  await page.waitForURL(`**/knowledge/${PLAYBOOKS_ID}/settings`);

  await page.getByRole("button", { name: "Full screen", exact: true }).click();
  await page.waitForURL(/\/chats$/);
  await expect(shell).toHaveAttribute("data-layout", "chat");

  await page.goBack();
  await page.waitForURL(`**/knowledge/${PLAYBOOKS_ID}/settings`);
  await expect(shell).toHaveAttribute("data-layout", "beside");
  await page.goBack();
  await page.waitForURL(`**/knowledge/${PLAYBOOKS_ID}/sources`);
  await page.goForward();
  await page.waitForURL(`**/knowledge/${PLAYBOOKS_ID}/settings`);

  await page.goto(`/threads/${threadId}`);
  await expect(shell).toHaveAttribute("data-layout", "beside");
  await expect(page.getByRole("region", { name: "Platform Chat", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await page.waitForURL(`**/knowledge/${PLAYBOOKS_ID}/settings`);
  await page.goForward();
  await page.waitForURL(`**/threads/${threadId}`);
});
