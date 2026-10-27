import { expect, test, type Page } from "@playwright/test";

// Knowledge pages against a mocked API: the tabbed landing (collections and
// search accounts) and a collection browsed with the shared file browser.

const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";
const DOCS_ID = "kn_019fda06ccf67d618776620713254657";
const PLAYBOOKS_ID = "kn_019fda06ccf67d618776620713254658";
const MANAGED_SOURCE = "0199c0de-0000-7000-8000-000000000001";
const INDEXER_ID = "0199c0de-0000-7000-8000-0000000000aa";

const docs = {
  id: DOCS_ID,
  name: "everruns-docs",
  description: "Built-in public Everruns documentation; read and grep without credentials",
  indexing: "off",
  status: "active",
  revision: 1,
  published_revision: 1,
  indexer_id: null,
  embedding_model_id: null,
  rules: {},
  file_count: 3,
  indexed_count: 0,
  jobs: [],
  sources: [
    {
      id: "0199c0de-0000-7000-8000-000000000002",
      kind: "builtin",
      root_path: "/",
      config: {},
      sync_status: "synced",
      last_error: null,
      last_synced_at: "2026-10-27T13:00:00Z",
    },
  ],
};

const playbooks = {
  ...docs,
  id: PLAYBOOKS_ID,
  name: "support-playbooks",
  description: "Escalation runbooks and canned answers for the support agent.",
  indexing: "hybrid",
  indexer_id: INDEXER_ID,
  file_count: 2,
  indexed_count: 2,
  sources: [
    {
      id: MANAGED_SOURCE,
      kind: "managed",
      root_path: "/",
      config: {},
      sync_status: "synced",
      last_error: null,
      last_synced_at: null,
    },
  ],
};

const file = (path: string, sourceId: string, extraction = "ready") => ({
  kind: "file",
  name: path.split("/").pop(),
  id: `file-${path}`,
  path,
  current_version: "0199c0de-0000-7000-8000-0000000000f1",
  indexed_version: null,
  indexable: true,
  readable: true,
  source_id: sourceId,
  version: {
    size_bytes: 7307,
    mime_type: "text/markdown",
    extraction_status: extraction,
    metadata: {},
    warnings: [],
    created_at: "2026-10-24T10:00:00Z",
  },
});

const TREES: Record<string, Record<string, unknown[]>> = {
  [DOCS_ID]: {
    "/": [
      { kind: "folder", path: "/advanced", name: "advanced" },
      file("/index.md", docs.sources[0].id),
    ],
    "/advanced": [
      file("/advanced/budgets.md", docs.sources[0].id),
      file("/advanced/diagram.bin", docs.sources[0].id, "unsupported"),
    ],
  },
  [PLAYBOOKS_ID]: {
    "/": [file("/refund-policy.md", MANAGED_SOURCE), file("/vip-escalation.md", MANAGED_SOURCE)],
  },
};

async function mockApi(page: Page) {
  await page.route("**/api/v1/**", async (route) => {
    const url = new URL(route.request().url());
    const pathname = url.pathname;
    let json: unknown = { data: [], has_more: false, total: 0 };

    if (pathname === "/api/v1/auth/config") {
      json = { mode: "none", password_auth_enabled: false, oauth_providers: [], signup_enabled: false };
    } else if (pathname === "/api/v1/auth/me") {
      json = {
        id: "user-1",
        email: "dev@example.com",
        name: "Dev User",
        roles: [],
        email_verified: true,
        organizations: [{ public_id: DEFAULT_ORG_ID, name: "Default", role: "owner" }],
      };
    } else if (pathname.endsWith("/feature-flags")) {
      json = { knowledge: true };
    } else if (pathname.endsWith("/switch-org")) {
      json = { success: true, org_id: DEFAULT_ORG_ID };
    } else if (/^\/api\/v1\/orgs\/[^/]+$/.test(pathname)) {
      json = { id: DEFAULT_ORG_ID, name: "Default" };
    } else if (pathname === "/api/v1/knowledge") {
      json = { data: [docs, playbooks], next_cursor: null };
    } else if (pathname === "/api/v1/knowledge-indexers") {
      json = {
        data: [
          {
            id: INDEXER_ID,
            name: "Turbopuffer prod",
            endpoint: "https://gcp-us-central1.turbopuffer.com",
            credential_binding_id: null,
            status: "active",
          },
        ],
      };
    } else if (pathname.endsWith("/files/content")) {
      await route.fulfill({
        contentType: "text/markdown",
        body: `# ${url.searchParams.get("path")}\n\nBudgets cap what a Session may spend.`,
      });
      return;
    } else {
      const match = pathname.match(/^\/api\/v1\/knowledge\/([^/]+)(\/files)?$/);
      if (match) {
        const collection = match[1] === DOCS_ID ? docs : playbooks;
        if (match[2]) {
          const path = (url.searchParams.get("path") ?? "/").replace(/(.)\/$/, "$1");
          json = { data: TREES[collection.id][path] ?? [], next_cursor: null };
        } else {
          json = collection;
        }
      }
    }
    await route.fulfill({ json });
  });
}

test.describe("Knowledge pages", () => {
  test.beforeEach(async ({ context, page, baseURL }) => {
    const origin = new URL(baseURL ?? "http://localhost:9100");
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: origin.hostname, path: "/" },
    ]);
    await mockApi(page);
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
