// Regression test for EVE-784: a cold dashboard load must issue each distinct
// agent-list query exactly once, and a window focus/visibility event must not
// immediately repeat those (still-fresh) requests.
//
// Root cause guarded here: org-scoped CRUD queries used to pass an explicit
// `staleTime: undefined`, which overwrote the QueryClient default (60_000) and
// left every agent-list query immediately stale. With `refetchOnWindowFocus`,
// the first focus/visibilitychange after load (e.g. CDP/DevTools attaching)
// duplicated the whole agent-list fetch pair.

import { render, waitFor } from "@testing-library/react";
import { ReactNode } from "react";
import { QueryClientProvider, focusManager } from "@tanstack/react-query";
import { createAppQueryClient } from "@/lib/query-client";
import { OrgProvider, useOrg } from "@/providers/org-provider";
import { useAgents } from "@/hooks/use-agents";
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

let agentCalls: string[] = [];

beforeEach(() => {
  agentCalls = [];
  mockSwitchOrg.mockClear().mockResolvedValue(undefined);
  jest.spyOn(Storage.prototype, "getItem").mockReturnValue(null);
  jest.spyOn(Storage.prototype, "setItem").mockImplementation(() => {});
  global.fetch = jest.fn(async (input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input.toString();
    if (url.includes("/v1/agents")) agentCalls.push(url);
    return {
      ok: true,
      status: 200,
      statusText: "OK",
      text: async () => JSON.stringify({ data: [] }),
    } as Response;
  }) as unknown as typeof fetch;
});

afterEach(() => {
  focusManager.setFocused(undefined);
});

// The dashboard asks for the active agent list and the archived-inclusive
// reference list — two distinct queries that should each run once.
function DashboardAgents() {
  useAgents();
  useAgents({ includeArchived: true });
  return <div>dashboard</div>;
}

// Mirror MainLayout: children mount only after org bootstrap finishes.
function Gate({ children }: { children: ReactNode }) {
  const { isLoading } = useOrg();
  return isLoading ? <div>loading</div> : <>{children}</>;
}

function renderDashboard() {
  const client = createAppQueryClient();
  render(<DashboardAgents />, {
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

const plainCalls = () => agentCalls.filter((u) => !u.includes("include_archived"));
const archivedCalls = () => agentCalls.filter((u) => u.includes("include_archived"));

describe("dashboard agent-list requests (EVE-784)", () => {
  it("cold load issues each distinct agent-list query exactly once", async () => {
    renderDashboard();

    await waitFor(() => {
      expect(plainCalls()).toHaveLength(1);
      expect(archivedCalls()).toHaveLength(1);
    });

    // Allow any stray refetch to settle, then confirm still exactly one each.
    await new Promise((r) => setTimeout(r, 50));
    expect(plainCalls()).toHaveLength(1);
    expect(archivedCalls()).toHaveLength(1);
  });

  it("a window focus regain does not repeat the fresh agent-list requests", async () => {
    renderDashboard();

    await waitFor(() => {
      expect(plainCalls()).toHaveLength(1);
      expect(archivedCalls()).toHaveLength(1);
    });
    await new Promise((r) => setTimeout(r, 20));

    // Simulate the browser regaining focus shortly after the cold load.
    focusManager.setFocused(false);
    focusManager.setFocused(true);
    await new Promise((r) => setTimeout(r, 60));

    // Data is fresh (60s client default), so focus must not refetch.
    expect(plainCalls()).toHaveLength(1);
    expect(archivedCalls()).toHaveLength(1);
  });

  it("explicit invalidation still refetches despite the fresh-data window", async () => {
    const client = renderDashboard();

    await waitFor(() => {
      expect(plainCalls()).toHaveLength(1);
      expect(archivedCalls()).toHaveLength(1);
    });

    // The staleTime fix must not suppress legitimate refetches: an
    // invalidation (as fired after agent create/update/archive, or on org
    // switch) marks queries stale regardless of staleTime and refetches.
    await client.invalidateQueries({ queryKey: ["agents"] });

    await waitFor(() => {
      expect(plainCalls()).toHaveLength(2);
      expect(archivedCalls()).toHaveLength(2);
    });
  });
});
