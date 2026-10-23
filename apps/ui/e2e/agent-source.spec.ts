import { test, expect } from "@playwright/test";

const agentId = "agent_019fd9b43fa37512b8f25226b21c2c8b";
const orgId = "org_00000000000000000000000000000001";
const commit = "a".repeat(40);

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
        enabled = true;
        source = {
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
    else if (p === `/agents/${agentId}/github`)
      json = {
        identity_id: "identity_1",
        connected: true,
        app_created: true,
        app_slug: "support",
        kind: "agent_app",
        missing_permissions: [],
        suggested_owners: [],
        everruns_app_installations: [],
      };
    else if (p.endsWith("/connections/github/repositories"))
      json = [
        { full_name: "acme/agents", private: true, html_url: "https://github.com/acme/agents" },
      ];
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
  await page.getByRole("button", { name: "Sync from GitHub", exact: true }).click();
  await expect(page.getByRole("button", { name: "Enable continuous sync" })).toBeDisabled();
  await page.getByLabel("Repository", { exact: true }).selectOption("acme/agents");
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
  await page.getByRole("button", { name: "Disconnect", exact: true }).click();
  await page.getByRole("button", { name: "Disconnect source sync", exact: true }).click();
  await expect(page.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
  await expect(page.getByText("Help with support requests.", { exact: true })).toBeVisible();
});
