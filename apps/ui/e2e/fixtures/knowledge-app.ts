import type { Page } from "@playwright/test";

export const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";
export const DOCS_ID = "kn_019fda06ccf67d618776620713254657";
export const PLAYBOOKS_ID = "kn_019fda06ccf67d618776620713254658";
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

export async function mockKnowledgeApi(
  page: Page,
  options: { chat?: boolean; work?: boolean; role?: "owner" | "admin" | "member" } = {},
) {
  const model = {
    id: "model_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    model_id: "meta/muse-spark-1.3-contributor",
    display_name: "Muse 1.3 Contributor",
    provider_id: "provider_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    provider_name: "OpenRouter",
    provider_type: "openrouter",
    enabled: true,
    healthy: true,
    capabilities: [],
    service: "chat",
  };
  await page.route("**/api/v1/**", async (route) => {
    const url = new URL(route.request().url());
    const pathname = url.pathname;
    let json: unknown = { data: [], has_more: false, total: 0 };

    if (pathname === "/api/v1/auth/config") {
      json = {
        mode: "none",
        password_auth_enabled: false,
        oauth_providers: [],
        signup_enabled: false,
      };
    } else if (pathname === "/api/v1/auth/me") {
      json = {
        id: "user-1",
        email: "dev@example.com",
        name: "Dev User",
        roles: [],
        email_verified: true,
        organizations: [
          { public_id: DEFAULT_ORG_ID, name: "Default", role: options.role ?? "owner" },
        ],
      };
    } else if (pathname.endsWith("/feature-flags")) {
      json = {
        knowledge: true,
        chat_mode: options.chat ?? false,
        chat_threads: options.work ?? false,
      };
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
    } else if (
      pathname === "/api/v1/sessions/platform-chat" ||
      pathname === "/api/v1/sessions/session_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ) {
      json = {
        id: "session_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        organization_id: DEFAULT_ORG_ID,
        agent_id: "agent_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        harness_id: "harness_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        model_id: model.id,
        status: "idle",
        source: "chat",
        tags: ["chat", "platform-chat-starter"],
        title: "Chat",
        parent_session_id: null,
        created_at: "2026-10-27T10:00:00Z",
        updated_at: "2026-10-27T10:00:00Z",
      };
    } else if (pathname === "/api/v1/agents/agent_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa") {
      json = {
        id: "agent_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        name: "platform-chat",
        display_name: "Chat",
        system_prompt: "Fictional UI fixture",
        capabilities: [],
        tags: [],
        starters: [],
        initial_files: [],
        status: "active",
      };
    } else if (pathname === "/api/v1/models") {
      json = { data: [model], has_more: false, total: 1 };
    } else if (pathname === `/api/v1/models/${model.id}`) {
      json = model;
    } else if (pathname.endsWith("/resolved-model")) {
      json = { model_id: model.id };
    } else if (pathname.endsWith("/participants") || pathname.endsWith("/tasks")) {
      json = [];
    } else if (pathname.endsWith("/config")) {
      json = { policies: {} };
    } else if (pathname.startsWith("/api/v1/history/")) {
      json = [];
    } else if (pathname.endsWith("/events") || pathname.endsWith("/messages")) {
      json = { data: [], has_more: false };
    } else if (pathname.endsWith("/sse")) {
      await route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: ": fixture idle\n\n",
      });
      return;
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
