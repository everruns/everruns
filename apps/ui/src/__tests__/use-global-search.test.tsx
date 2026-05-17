import { act, renderHook } from "@testing-library/react";
import { useGlobalSearch } from "@/hooks/use-global-search";
import type { OrganizationMembership } from "@/lib/api/types";

jest.mock("@/hooks/use-agents", () => ({
  useAgents: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-sessions", () => ({
  useSessions: () => ({ data: { data: [] } }),
}));
jest.mock("@/hooks/use-harnesses", () => ({
  useHarnesses: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-skills", () => ({
  useSkills: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-mcp-servers", () => ({
  useMcpServers: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-capabilities", () => ({
  useCapabilities: () => ({ data: [] }),
  useDeclarativeCapabilities: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-evals", () => ({
  useEvals: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-apps", () => ({
  useApps: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-agent-identities", () => ({
  useAgentIdentities: () => ({ data: [] }),
}));

const mockSetCurrentOrg = jest.fn();

const mockCurrentOrg: OrganizationMembership = {
  public_id: "org_current",
  name: "Current Org",
  role: "owner",
};

const mockSecondOrg: OrganizationMembership = {
  public_id: "org_second",
  name: "Second Org",
  role: "member",
};

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({
    currentOrg: mockCurrentOrg,
    organizations: [mockCurrentOrg, mockSecondOrg],
    setCurrentOrg: mockSetCurrentOrg,
  }),
}));

describe("useGlobalSearch", () => {
  beforeEach(() => {
    mockSetCurrentOrg.mockClear();
  });

  it("finds organizations and switches to the selected organization", () => {
    const { result } = renderHook(() => useGlobalSearch("second"));

    const orgResult = result.current.find((item) => item.category === "organization");

    expect(orgResult).toMatchObject({
      id: "organization:org_second",
      title: "Second Org",
      subtitle: "Switch organization > org_second",
    });

    act(() => {
      orgResult?.onSelect?.();
    });

    expect(mockSetCurrentOrg).toHaveBeenCalledWith(mockSecondOrg);
  });

  it("keeps the previous British spelling as a search alias", () => {
    const { result } = renderHook(() => useGlobalSearch("organisation"));

    expect(result.current.some((item) => item.href === "/settings/organization")).toBe(true);
    expect(result.current.some((item) => item.id === "organization:org_second")).toBe(true);
  });

  it("marks the current organization without attaching a switch action", () => {
    const { result } = renderHook(() => useGlobalSearch("current"));

    const orgResult = result.current.find((item) => item.category === "organization");

    expect(orgResult).toMatchObject({
      id: "organization:org_current",
      title: "Current Org",
      subtitle: "Current organization > org_current",
    });
    expect(orgResult?.onSelect).toBeUndefined();
  });
});
