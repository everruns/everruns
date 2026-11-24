import { expect, test } from "@playwright/test";
import { DOCS_ID, mockKnowledgeApi } from "./fixtures/knowledge-app";

test.use({ viewport: { width: 1400, height: 950 }, deviceScaleFactor: 2 });

test("chat explains catalog MCP blockers without mistaking them for sign-in", async ({
  page,
  context,
  baseURL,
}, testInfo) => {
  test.setTimeout(90_000);
  await context.addCookies([
    { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
  ]);
  await mockKnowledgeApi(page);
  await page.route("**/api/v1/platform/sessions/*/mcp-servers", (route) =>
    route.fulfill({
      json: [
        {
          name: "Work tracker",
          url: "https://work-tracker.example.test/mcp",
          catalog_name: "example-preset",
          connection: "not_needed",
          discovery: "preset_disabled",
        },
        {
          name: "Team calendar",
          url: "https://team-calendar.example.test/mcp",
          catalog_name: "example-preset",
          connection: "not_needed",
          discovery: "preset_unavailable",
        },
        {
          name: "Reference search",
          url: "https://reference-search.example.test/mcp",
          catalog_name: "example-preset",
          connection: "not_connected",
          discovery: "authentication_required",
        },
      ],
    }),
  );
  await page.goto(`/knowledge/${DOCS_ID}`);
  await page.getByRole("button", { name: "MCP servers in this chat (3)" }).click();

  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText(/catalog preset is disabled/)).toBeVisible();
  await expect(dialog.getByText(/catalog preset is unavailable/)).toBeVisible();
  await expect(dialog.getByText("Reference search")).toBeVisible();
  await expect(dialog.getByText("Needs sign-in", { exact: true })).toHaveCount(1);
  await expect(dialog.getByText("Tools need sign-in", { exact: true })).toHaveCount(1);
  await testInfo.attach("Chat MCP diagnostics", {
    body: await dialog.screenshot({ animations: "disabled" }),
    contentType: "image/png",
  });
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});
