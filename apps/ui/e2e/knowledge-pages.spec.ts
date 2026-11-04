import { expect, test } from "@playwright/test";

// Knowledge pages against a mocked API: the tabbed landing (collections and
// search accounts) and a collection browsed with the shared file browser.

import { DEFAULT_ORG_ID, DOCS_ID, PLAYBOOKS_ID, mockKnowledgeApi } from "./fixtures/knowledge-app";

test.describe("Knowledge pages", () => {
  test.beforeEach(async ({ context, page, baseURL }) => {
    const origin = new URL(baseURL ?? "http://localhost:9100");
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: origin.hostname, path: "/" },
    ]);
    await mockKnowledgeApi(page);
  });

  test("landing has collections and accounts tabs", async ({ page }) => {
    await page.goto("/knowledge");
    const tabs = page.getByRole("navigation", { name: "Knowledge sections" });
    await expect(tabs.getByRole("link", { name: "Collections" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    await expect(page.getByRole("link", { name: "support-playbooks" })).toHaveAttribute(
      "href",
      `/knowledge/${PLAYBOOKS_ID}`,
    );
    await expect(page.getByText("Hybrid search")).toBeVisible();

    await tabs.getByRole("link", { name: "Accounts" }).click();
    await expect(page).toHaveURL(/\/knowledge\/accounts$/, { timeout: 30_000 });
    await expect(page.getByText("Turbopuffer prod")).toBeVisible();
    await expect(page.getByText("Used by support-playbooks")).toBeVisible();
    await expect(page.getByRole("button", { name: "Add search account" })).toBeVisible();
  });

  test("collection files browse as a tree and open in the viewer", async ({ page }) => {
    await page.goto(`/knowledge/${DOCS_ID}`);
    await expect(page.getByRole("heading", { name: "everruns-docs" })).toBeVisible();
    const tree = page.getByRole("tree");
    await tree.getByText("advanced", { exact: true }).click();
    await tree.getByText("budgets.md").click();
    await expect(page).toHaveURL(/file=%2Fadvanced%2Fbudgets\.md/);
    await expect(page.getByText("Budgets cap what a Session may spend.")).toBeVisible();
    // The built-in docs are read-only: no upload and no delete.
    await expect(page.getByRole("button", { name: "Upload files" })).toHaveCount(0);
    await expect(page.getByTitle("Delete")).toHaveCount(0);
    await expect(page.getByText("Agent can read")).toBeVisible();
  });

  test("managed collection offers uploads and deletes", async ({ page }) => {
    await page.goto(`/knowledge/${PLAYBOOKS_ID}?file=%2Fvip-escalation.md`);
    await expect(page.getByRole("button", { name: "Upload files" })).toBeVisible();
    await expect(page.getByTitle("Delete").first()).toBeVisible();
    await expect(page.getByText("Include in index")).toBeVisible();
  });

  test("search tab explains indexing when it is off", async ({ page }) => {
    await page.goto(`/knowledge/${DOCS_ID}/search`);
    await expect(page.getByText("Indexing is off")).toBeVisible();
    await expect(page.getByRole("link", { name: "Open settings" })).toHaveAttribute(
      "href",
      `/knowledge/${DOCS_ID}/settings`,
    );
  });

  test("sources tab lists where files come from", async ({ page }) => {
    await page.goto(`/knowledge/${PLAYBOOKS_ID}/sources`);
    await expect(page.getByRole("cell", { name: /Managed files/ })).toBeVisible();
    await expect(page.getByRole("button", { name: "Connect a source" })).toBeVisible();
  });
});
