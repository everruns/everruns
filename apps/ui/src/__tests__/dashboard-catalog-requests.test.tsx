// Regression test for EVE-783: the dashboard must not overfetch the capability
// and model catalogs when the rendered state cannot display them.
//
// Root cause guarded here: the dashboard called useCapabilities() and
// useModels() unconditionally, so an empty org downloaded ~228 KB of
// capabilities and ~78 KB of models that nothing could render. The fix gates
// each catalog query on the data that actually consumes it:
//   - capabilities: enabled only when a displayed active agent references one
//   - models: enabled only when a recent session references a model
//
// These tests render the real dashboard page against a fetch stub and assert
// the exact catalog request set for empty, agents-only, sessions-only, and
// populated dashboards.

import { render, waitFor } from "@testing-library/react";
import { ReactNode } from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import { createAppQueryClient } from "@/lib/query-client";
import { OrgProvider, useOrg } from "@/providers/org-provider";
import DashboardPage from "@/app/(main)/dashboard/page";
import type { OrganizationMembership } from "@/lib/api/types";

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: jest.fn(), replace: jest.fn(), back: jest.fn() }),
  usePathname: () => "/dashboard",
}));

const mockSwitchOrg = jest.fn().mockResolvedValue(undefined);
jest.mock("@/lib/api/users", () => ({
  switchOrg: (...args: unknown[]) => mockSwitchOrg(...args),
}));

const ORG: OrganizationMembership = {
  public_id: "org_00000000000000000000000000000001",
  name: "Default Org",
  role: "owner",
};

// Real OrgProvider is used; only the auth source is stubbed.
jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => ({ user: { organizations: [ORG] }, isLoading: false }),
}));

type Dataset = {
  agents: unknown[];
  sessions: unknown[];
};

let requested: string[] = [];
let dataset: Dataset = { agents: [], sessions: [] };

function jsonResponse(body: unknown): Response {
  return {
    ok: true,
    status: 200,
    statusText: "OK",
    text: async () => JSON.stringify(body),
  } as Response;
}

beforeEach(() => {
  requested = [];
  dataset = { agents: [], sessions: [] };
  mockSwitchOrg.mockClear().mockResolvedValue(undefined);
  jest.spyOn(Storage.prototype, "getItem").mockReturnValue(null);
  jest.spyOn(Storage.prototype, "setItem").mockImplementation(() => {});

  global.fetch = jest.fn(async (input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input.toString();
    requested.push(url);

    // Order matters: check the more specific paths first.
    if (url.includes("/v1/capabilities")) return jsonResponse({ data: [] });
    if (url.includes("/v1/models")) return jsonResponse({ data: [] });
    if (url.includes("/v1/sessions/stats")) {
      return jsonResponse({ total: dataset.sessions.length, active: 0, idle: 0 });
    }
    if (url.includes("/v1/sessions")) {
      return jsonResponse({
        data: dataset.sessions,
        total: dataset.sessions.length,
        offset: 0,
        limit: 5,
      });
    }
    if (url.includes("/v1/agents")) return jsonResponse({ data: dataset.agents });
    return jsonResponse({ data: [] });
  }) as unknown as typeof fetch;
});

// Mirror MainLayout: children mount only after org bootstrap finishes.
function Gate({ children }: { children: ReactNode }) {
  const { isLoading } = useOrg();
  return isLoading ? <div>loading</div> : <>{children}</>;
}

function renderDashboard() {
  const client = createAppQueryClient();
  render(<DashboardPage />, {
    wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>
        <OrgProvider initialOrgId={null}>
          <Gate>{children}</Gate>
        </OrgProvider>
      </QueryClientProvider>
    ),
  });
  return client;
}

const capabilityCalls = () => requested.filter((u) => u.includes("/v1/capabilities"));
const modelCalls = () => requested.filter((u) => u.includes("/v1/models"));
const agentCalls = () => requested.filter((u) => u.includes("/v1/agents"));

async function waitForAgentsLoaded() {
  // The dashboard always fetches the agent lists; once they land the gating
  // logic has run against real data.
  await waitFor(() => expect(agentCalls().length).toBeGreaterThanOrEqual(2));
  // Let any gated catalog query fire and settle.
  await new Promise((r) => setTimeout(r, 60));
}

const agentWithCapability = {
  id: "agent_1",
  name: "Cap Agent",
  status: "active",
  capabilities: [{ ref: "cap_1" }],
};

const agentWithoutCapability = {
  id: "agent_2",
  name: "Plain Agent",
  status: "active",
  capabilities: [],
};

const sessionWithModel = {
  id: "session_1",
  status: "idle",
  model_id: "model_1",
  created_at: "2024-01-01T00:00:00Z",
  usage: { input_tokens: 0, output_tokens: 0 },
};

const sessionWithoutModel = {
  id: "session_2",
  status: "idle",
  model_id: null,
  created_at: "2024-01-01T00:00:00Z",
  usage: { input_tokens: 0, output_tokens: 0 },
};

describe("dashboard catalog requests (EVE-783)", () => {
  it("empty dashboard requests neither capabilities nor models", async () => {
    dataset = { agents: [], sessions: [] };
    renderDashboard();
    await waitForAgentsLoaded();

    expect(capabilityCalls()).toHaveLength(0);
    expect(modelCalls()).toHaveLength(0);
  });

  it("agents-only with a capability reference fetches capabilities once, no models", async () => {
    dataset = { agents: [agentWithCapability], sessions: [] };
    renderDashboard();
    await waitForAgentsLoaded();

    await waitFor(() => expect(capabilityCalls()).toHaveLength(1));
    expect(modelCalls()).toHaveLength(0);
  });

  it("agents without capability references do not fetch the capability catalog", async () => {
    dataset = { agents: [agentWithoutCapability], sessions: [] };
    renderDashboard();
    await waitForAgentsLoaded();

    expect(capabilityCalls()).toHaveLength(0);
    expect(modelCalls()).toHaveLength(0);
  });

  it("sessions-only with a model reference fetches models once, no capabilities", async () => {
    dataset = { agents: [], sessions: [sessionWithModel] };
    renderDashboard();
    await waitForAgentsLoaded();

    await waitFor(() => expect(modelCalls()).toHaveLength(1));
    expect(capabilityCalls()).toHaveLength(0);
  });

  it("sessions without model references do not fetch the model catalog", async () => {
    dataset = { agents: [], sessions: [sessionWithoutModel] };
    renderDashboard();
    await waitForAgentsLoaded();

    expect(modelCalls()).toHaveLength(0);
    expect(capabilityCalls()).toHaveLength(0);
  });

  it("populated dashboard fetches each catalog exactly once", async () => {
    dataset = { agents: [agentWithCapability], sessions: [sessionWithModel] };
    renderDashboard();
    await waitForAgentsLoaded();

    await waitFor(() => {
      expect(capabilityCalls()).toHaveLength(1);
      expect(modelCalls()).toHaveLength(1);
    });

    // Allow any stray refetch to settle, then confirm still exactly one each.
    await new Promise((r) => setTimeout(r, 60));
    expect(capabilityCalls()).toHaveLength(1);
    expect(modelCalls()).toHaveLength(1);
  });
});
