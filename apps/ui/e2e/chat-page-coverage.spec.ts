import { expect, test } from "@playwright/test";
import { DOCS_ID, PLAYBOOKS_ID, mockKnowledgeApi } from "./fixtures/knowledge-app";

for (const role of ["owner", "admin", "member"] as const) {
  test(`new Tracing UI respects ${role} access beside Chat`, async ({ context, page, baseURL }) => {
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
    ]);
    await mockKnowledgeApi(page, { chat: true, role });
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
    await page.keyboard.press("Control+j");
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
for (const chat of [false, true])
  for (const work of [false, true]) {
    test(`new Knowledge UI with workspace=${chat} background work=${work}`, async ({
      context,
      page,
      baseURL,
    }) => {
      await context.addCookies([
        { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
      ]);
      await mockKnowledgeApi(page, { chat, work });
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(`/knowledge/${PLAYBOOKS_ID}?file=%2Fvip-escalation.md`);
      await expect(
        page.getByRole("heading", { name: "support-playbooks", exact: true }),
      ).toBeVisible();
      await expect(page.getByText("Budgets cap what a Session may spend.")).toBeVisible();
      await page.keyboard.press("Control+j");
      if (chat) {
        await expect(page.locator(".chat-mode-shell")).toHaveAttribute("data-chat-mode", "open");
        await expect(page.getByRole("region", { name: "Current page" })).toBeVisible();
        await expect(
          page.getByRole("region", { name: "Platform Chat", exact: true }),
        ).toBeVisible();
        await expect(page.getByRole("button", { name: "Back to app", exact: true })).toBeVisible();
        await expect(page.getByRole("heading", { name: "Thread not found" })).toHaveCount(0);
        const nav = page.getByRole("navigation", { name: "Collection sections" });
        await nav.getByRole("link", { name: "Sources" }).click();
        await expect(page.getByRole("cell", { name: /Managed files/ })).toBeVisible();
        await nav.getByRole("link", { name: "Settings" }).click();
        const name = page.getByRole("textbox", { name: "Name", exact: true });
        await expect(name).toHaveValue("support-playbooks");
        await name.fill("Support draft");
        await page.keyboard.press("Control+j");
        await expect(page.locator(".chat-mode-shell")).toHaveAttribute("data-chat-mode", "closed");
        await expect(name).toHaveValue("Support draft");
        await page.keyboard.press("Control+j");
        await expect(name).toHaveValue("Support draft");
      } else await expect(page.locator(".chat-mode-shell")).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  }

for (const role of ["owner", "admin", "member"] as const) {
  test(`new Knowledge settings and source controls respect ${role}`, async ({
    context,
    page,
    baseURL,
  }) => {
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
    ]);
    await mockKnowledgeApi(page, { chat: true, role });
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
    await mockKnowledgeApi(page, { chat: true });
    await page.setViewportSize({ width: 1200, height: 1000 });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto(`/knowledge/${PLAYBOOKS_ID}?file=%2Fvip-escalation.md`);
    await expect(page.getByText("Budgets cap what a Session may spend.")).toBeVisible();
    await page.keyboard.press("Control+j");
    await expect(page.locator(".chat-mode-shell")).toHaveAttribute("data-chat-mode", "open");
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
    await page.setViewportSize({ width: 390, height: 844 });
    await page.getByRole("button", { name: "Back to app", exact: true }).click();
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
