import { test, expect } from "@playwright/test";

const agentId = "agent_019fd9b43fa37512b8f25226b21c2c8b";
const orgId = "org_00000000000000000000000000000001";
const commit = "a".repeat(40);
test.use({ deviceScaleFactor: 2 });

test("reviews a GitHub package, locks editing, and disconnects without losing the Agent", async ({
  page,
  context,
  baseURL,
}) => {
  await context.addCookies([
    { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
  ]);
  let source: Record<string, unknown> | null = null;
  let enabled = false;
  let syncCalls = 0;
  let failSync = true;
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const p = new URL(request.url()).pathname.replace("/api/v1", "");
    let json: unknown = { data: [], total: 0, has_more: false };
    if (p === "/auth/config")
      json = {
        mode: "none",
        password_auth_enabled: false,
        oauth_providers: [],
        signup_enabled: false,
      };
    else if (p === "/auth/me")
      json = {
        id: "user-1",
        email: "dev@example.com",
        name: "Dev",
        email_verified: true,
        roles: [],
        organizations: [{ public_id: orgId, name: "Default", role: "owner" }],
      };
    else if (p.endsWith("/feature-flags")) json = {};
    else if (p.endsWith("/switch-org")) json = { success: true, org_id: orgId };
    else if (p.endsWith("/config"))
      json = { policies: { "agent.manage": true, "agent.dangerous": true } };
    else if (p === `/agents/${agentId}`)
      json = {
        id: agentId,
        name: "support",
        display_name: "Support Agent",
        system_prompt: enabled ? "Help with support requests." : "Original instructions.",
        status: "active",
        harness_id: "harness_test",
        capabilities: [],
        tags: [],
        initial_files: [],
        session_count: 0,
        created_at: "2026-10-19T12:00:00Z",
        updated_at: "2026-10-19T12:00:00Z",
      };
    else if (p === `/agents/${agentId}/source`) {
      if (request.method() === "PUT") {
        const body = request.postDataJSON();
        expect(body.commit).toBe(commit);
        expect(body.path).toBe("agents/support");
        expect(body.agent_digest).toBe("reviewed-base");
        expect(body.mode).toBe("pull");
        expect(body.github_account).toBe("personal");
        enabled = true;
        source = {
          mode: "pull",
          local_dirty: false,
          generation: "g1",
          repository: "acme/agents",
          branch: "main",
          path: "agents/support",
          managed_channels: [],
          applied_commit: commit,
          status: "synced",
          error: null,
          last_checked_at: "2026-10-19T12:00:00Z",
          last_synced_at: "2026-10-19T12:00:00Z",
        };
      } else if (request.method() === "DELETE") source = null;
      json = source;
    } else if (p === `/agents/${agentId}/source/sync`) {
      syncCalls++;
      await new Promise((resolve) => setTimeout(resolve, 300));
      if (failSync) {
        failSync = false;
        await route.fulfill({ status: 500, json: { message: "Temporary sync failure" } });
        return;
      }
      source = { ...source, status: "pending" };
      json = source;
    } else if (p === `/agents/${agentId}/source/options`)
      json = {
        default_branch: "main",
        branch: "main",
        branches: ["main", "staging"],
        paths: ["agents/"],
      };
    else if (p === `/agents/${agentId}/source/preview`)
      json = {
        valid: true,
        commit,
        agent_digest: "reviewed-base",
        diagnostics: [],
        preview: {
          name: "support",
          instructions: "Help with support requests.",
          harness: "base",
          capabilities: {},
          files: {},
        },
        changes: [
          {
            path: "/instructions",
            before: "Original instructions.",
            after: "Help with support requests.",
          },
        ],
      };
    else if (p === `/agents/${agentId}/source/accounts`)
      json = {
        available: true,
        install_url: null,
        personal: {
          connected: true,
          login: "dev",
          repositories: [{ full_name: "acme/agents", account: "acme", private: true }],
          error: null,
        },
        organization: { can_connect: true, accounts: [], repositories: [], error: null },
      };
    else if (
      p === `/agents/${agentId}/channels` ||
      p === `/agents/${agentId}/triggers` ||
      p === `/agents/${agentId}/mcp-attachments` ||
      p.startsWith("/history/")
    )
      json = [];
    else if (p === `/agents/${agentId}/credentials`) json = { data: [] };
    else if (p.startsWith("/context/"))
      json = { entity_kind: "agent", entity_ref: agentId, content: "", revision: 0 };
    else if (p === `/orgs/${orgId}`) json = { id: orgId, name: "Default" };
    await route.fulfill({ json });
  });
  await page.goto(`/agents/${agentId}`);
  await expect(page.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "Agent source" })).toHaveCount(0);
  const menu = page.getByRole("button", { name: "More actions for Support Agent" });
  if (process.env.UI_EVIDENCE_DIR)
    await page
      .locator('[data-slot="page-masthead"]')
      .screenshot({ path: `${process.env.UI_EVIDENCE_DIR}/agent-actions.png` });
  await menu.focus();
  await page.keyboard.press("Enter");
  await page.getByRole("menuitem", { name: "Sync with GitHub", exact: true }).focus();
  await page.keyboard.press("Enter");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(menu).toBeFocused();
  await menu.click();
  if (process.env.UI_EVIDENCE_DIR)
    await page.screenshot({
      path: `${process.env.UI_EVIDENCE_DIR}/agent-menu.png`,
      clip: { x: 260, y: 100, width: 1000, height: 285 },
    });
  await page.getByRole("menuitem", { name: "Sync with GitHub", exact: true }).click();
  await expect(page.getByRole("button", { name: "Enable continuous sync" })).toBeDisabled();
  await page.getByLabel("Repository", { exact: true }).click();
  await page.getByRole("option", { name: "acme/agents", exact: true }).click();
  await expect(page.getByLabel("Branch", { exact: true })).toHaveValue("main");
  await page.getByLabel("Package folder or manifest").fill("agents/support");
  await page.getByRole("button", { name: "Preview changes", exact: true }).click();
  await expect(page.getByRole("button", { name: "Enable continuous sync" })).toBeEnabled();
  await page.getByRole("button", { name: "Enable continuous sync" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    page.getByText("Managed by GitHub. Edit the repository to update this Agent."),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Edit", exact: true })).toHaveCount(0);
  await expect(page.getByText("aaaaaaa", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Sync now", exact: true }).click();
  await expect(page.getByRole("button", { name: "Sync now", exact: true })).toBeDisabled();
  await expect(page.getByRole("alert").filter({ hasText: "Temporary sync failure" })).toContainText(
    "Temporary sync failure",
  );
  await expect(page.getByText("aaaaaaa", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Sync now", exact: true }).click();
  await expect(page.getByText("Queued", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Sync now", exact: true })).toBeDisabled();
  expect(syncCalls).toBe(2);
  await page.getByRole("button", { name: "Disconnect", exact: true }).click();
  await page.getByRole("button", { name: "Disconnect source sync", exact: true }).click();
  await expect(page.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
  await expect(page.getByText("Help with support requests.", { exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "Agent source" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Dry run", exact: true })).toHaveCount(0);
  // The development server may compile Playground during its first navigation.
  await Promise.all([
    page.waitForURL(`/playground/new?agent=${agentId}`),
    page.getByRole("button", { name: "Test in Playground", exact: true }).click(),
  ]);
});
