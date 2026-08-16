import { expect, test, type Page } from "@playwright/test";

const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";

async function installWebMcpHarness(page: Page) {
  await page.addInitScript(() => {
    type Tool = {
      name: string;
      description: string;
      inputSchema?: Record<string, unknown>;
      annotations?: Record<string, boolean>;
      execute(input: Record<string, unknown>): unknown;
    };
    const tools = new Map<string, Tool>();
    const modelContext = Object.assign(new EventTarget(), {
      async registerTool(tool: Tool, options?: { signal?: AbortSignal }) {
        tools.set(tool.name, tool);
        options?.signal?.addEventListener("abort", () => tools.delete(tool.name), { once: true });
      },
      async getTools() {
        return [...tools.values()]
          .sort((left, right) => left.name.localeCompare(right.name))
          .map(({ name, description, inputSchema, annotations }) => ({
            name,
            description,
            inputSchema: JSON.stringify(inputSchema ?? {}),
            annotations,
          }));
      },
      async executeTool(toolDescriptor: { name: string }, input: string) {
        const tool = tools.get(toolDescriptor.name);
        if (!tool) throw new Error(`Unknown tool: ${toolDescriptor.name}`);
        const result = await tool.execute(JSON.parse(input));
        return typeof result === "string" ? result : JSON.stringify(result);
      },
    });
    Object.defineProperty(document, "modelContext", { configurable: true, value: modelContext });
  });
}

async function mockAppApi(page: Page) {
  await page.route("**/api/v1/**", async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    let json: unknown;
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
        organizations: [{ public_id: DEFAULT_ORG_ID, name: "Default", role: "owner" }],
      };
    } else if (pathname.endsWith("/feature-flags")) {
      json = {
        notifications: false,
        evals: false,
        app_budgets: false,
        agent_versions: false,
        voice: false,
        agent_delegation: false,
        observers: false,
        public_chat: false,
        webmcp: true,
      };
    } else if (pathname.endsWith("/switch-org")) {
      json = { success: true, org_id: DEFAULT_ORG_ID };
    } else if (pathname.endsWith("/config")) {
      json = { policies: {} };
    } else if (pathname === "/api/v1/sessions/facets") {
      json = {
        active_now: 0,
        by_activity: [],
        by_agent: [],
        by_source: [],
        failed_today: 0,
        p95_duration_ms: 0,
        tokens_today: 0,
        total: 0,
      };
    } else {
      json = { data: [], has_more: false, total: 0 };
    }
    await route.fulfill({ json });
  });
}

test("registers and executes WebMCP shell tools", async ({
  context: browserContext,
  page,
  baseURL,
}) => {
  const origin = new URL(baseURL ?? "http://localhost:9100");
  await browserContext.addCookies([
    { name: "access_token", value: "e2e-only", domain: origin.hostname, path: "/" },
  ]);
  await installWebMcpHarness(page);
  await mockAppApi(page);
  await page.goto("/chats");
  await expect(page.getByRole("link", { name: "Sessions" })).toBeVisible();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (
          document.modelContext as unknown as {
            getTools(): Promise<Array<{ name: string }>>;
          }
        )
          .getTools()
          .then((tools) => tools.map((tool) => tool.name)),
      ),
    )
    .toEqual(expect.arrayContaining(["everruns_get_context", "everruns_search", "everruns_open"]));

  const context = await page.evaluate(async () => {
    const modelContext = document.modelContext as unknown as {
      getTools(): Promise<Array<{ name: string }>>;
      executeTool(tool: { name: string }, input: string): Promise<unknown>;
    };
    const tool = (await modelContext.getTools()).find(
      ({ name }) => name === "everruns_get_context",
    );
    if (!tool) throw new Error("everruns_get_context is not registered");
    return JSON.parse((await modelContext.executeTool(tool, "{}")) as string);
  });
  expect(context).toMatchObject({ organization_id: DEFAULT_ORG_ID, page: "chats" });

  await page.evaluate(async () => {
    const modelContext = document.modelContext as unknown as {
      getTools(): Promise<Array<{ name: string }>>;
      executeTool(tool: { name: string }, input: string): Promise<unknown>;
    };
    const tool = (await modelContext.getTools()).find(({ name }) => name === "everruns_open");
    if (!tool) throw new Error("everruns_open is not registered");
    await modelContext.executeTool(tool, JSON.stringify({ target_type: "page", page: "sessions" }));
  });
  await expect(page).toHaveURL(/\/sessions$/);
  await expect(page.getByText("Tokens today 0")).toBeVisible();
});
