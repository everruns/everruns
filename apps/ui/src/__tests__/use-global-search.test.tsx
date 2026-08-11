import { act, renderHook } from "@testing-library/react";
import { Blocks, BookOpen, Plug } from "lucide-react";
import { useGlobalSearch } from "@/hooks/use-global-search";
import type { OrganizationMembership } from "@/lib/api/types";

const mockUseAgents = jest.fn((_options?: { enabled?: boolean }) => ({ data: [] }));
jest.mock("@/hooks/use-agents", () => ({
  useAgents: (options?: { enabled?: boolean }) => mockUseAgents(options),
}));
const mockUseSessions = jest.fn(() => ({ data: { data: [] as Array<Record<string, unknown>> } }));
jest.mock("@/hooks/use-sessions", () => ({
  useSessions: () => mockUseSessions(),
}));
jest.mock("@/hooks/use-harnesses", () => ({
  useHarnesses: () => ({ data: [] }),
}));
const mockUseSkills = jest.fn((_options?: { enabled?: boolean }) => ({ data: [] }));
jest.mock("@/hooks/use-skills", () => ({
  useSkills: (options?: { enabled?: boolean }) => mockUseSkills(options),
}));
jest.mock("@/hooks/use-mcp-servers", () => ({
  useMcpServers: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-capabilities", () => ({
  useCapabilities: () => ({ data: [] }),
  useDeclarativeCapabilities: () => ({ data: [] }),
}));
const mockUseEvals = jest.fn((_options?: { enabled?: boolean }) => ({ data: [] }));
jest.mock("@/hooks/use-evals", () => ({
  useEvals: (options?: { enabled?: boolean }) => mockUseEvals(options),
}));
jest.mock("@/hooks/use-apps", () => ({
  useApps: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-agent-identities", () => ({
  useAgentIdentities: () => ({ data: [] }),
}));
const mockUseMemories = jest.fn((_options?: { enabled?: boolean }) => ({
  data: [
    {
      id: "mem_product",
      name: "Product Notes",
      description: "Product decisions",
    },
  ],
}));
jest.mock("@/hooks/use-memory", () => ({
  useMemories: (options?: { enabled?: boolean }) => mockUseMemories(options),
}));
const mockUseKnowledgeIndexes = jest.fn((_options?: { enabled?: boolean }) => ({
  data: [
    {
      id: "kidx_docs",
      name: "Documentation Index",
      description: "Published documentation",
    },
  ],
}));
jest.mock("@/hooks/use-knowledge-indexes", () => ({
  useKnowledgeIndexes: (options?: { enabled?: boolean }) => mockUseKnowledgeIndexes(options),
}));
const mockUseInstalledPlugins = jest.fn((_options?: { enabled?: boolean }) => ({
  data: [
    {
      id: "plugin_email",
      name: "email-service",
      display_name: "Email Service",
      description: "Transactional email",
    },
  ],
}));
jest.mock("@/hooks/use-plugins", () => ({
  useInstalledPlugins: (options?: { enabled?: boolean }) => mockUseInstalledPlugins(options),
}));
jest.mock("@/hooks/use-observers", () => ({
  useObservers: () => ({
    data: [
      {
        id: "observer_quality",
        name: "Answer Quality",
        description: "Scores production answers",
      },
    ],
  }),
}));
const mockUseSavedReports = jest.fn((_enabled?: boolean) => ({
  data: [
    {
      id: "report_weekly",
      name: "Weekly Usage",
      description: "Usage by week",
    },
  ],
}));
jest.mock("@/hooks/use-reporting", () => ({
  useSavedReports: (enabled?: boolean) => mockUseSavedReports(enabled),
}));

const mockFeatureFlags = {
  evals: false,
  skills: true,
  memory: true,
  knowledge: true,
  plugins: true,
  observers: true,
  machine_payments: true,
};
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlags: () => mockFeatureFlags,
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
    mockUseAgents.mockClear();
    mockUseSessions.mockReset();
    mockUseSessions.mockReturnValue({ data: { data: [] } });
    mockUseSkills.mockClear();
    mockUseEvals.mockClear();
    mockUseMemories.mockClear();
    mockUseKnowledgeIndexes.mockClear();
    mockUseInstalledPlugins.mockClear();
    mockUseSavedReports.mockClear();
    Object.assign(mockFeatureFlags, {
      evals: false,
      skills: true,
      memory: true,
      knowledge: true,
      plugins: true,
      observers: true,
      machine_payments: true,
    });
  });

  it("defers the evals fetch when the feature flag is off", () => {
    renderHook(() => useGlobalSearch(""));
    expect(mockUseEvals).toHaveBeenCalledWith({ enabled: false });
  });

  it("enables the evals fetch when the feature flag is on", () => {
    mockFeatureFlags.evals = true;
    renderHook(() => useGlobalSearch("eval"));
    expect(mockUseEvals).toHaveBeenCalledWith({ enabled: true });
  });

  it("does not fetch or reveal opt-in features when their flags are off", () => {
    Object.assign(mockFeatureFlags, {
      evals: false,
      skills: false,
      memory: false,
      knowledge: false,
      plugins: false,
    });

    const { result } = renderHook(() => useGlobalSearch("memory"));

    expect(mockUseSkills).toHaveBeenCalledWith({ enabled: false });
    expect(mockUseEvals).toHaveBeenCalledWith({ enabled: false });
    expect(mockUseMemories).toHaveBeenCalledWith({ enabled: false });
    expect(mockUseKnowledgeIndexes).toHaveBeenCalledWith({ enabled: false });
    expect(mockUseInstalledPlugins).toHaveBeenCalledWith({ enabled: false });
    expect(result.current).not.toContainEqual(expect.objectContaining({ href: "/memory" }));
    expect(result.current).not.toContainEqual(
      expect.objectContaining({ href: "/memory/mem_product" }),
    );
  });

  it("does not enable entity fetches before the user enters a query", () => {
    renderHook(() => useGlobalSearch(""));

    expect(mockUseAgents).toHaveBeenCalledWith({ enabled: false });
    expect(mockUseSavedReports).toHaveBeenCalledWith(false);
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

  it("opens session search results on the transcript", () => {
    mockUseSessions.mockReturnValue({
      data: {
        data: [
          {
            id: "session_123",
            title: "Recorded deployment",
            preview: "Deploy the service",
          },
        ],
      },
    });

    const { result } = renderHook(() => useGlobalSearch("recorded deployment"));

    expect(result.current).toContainEqual(
      expect.objectContaining({
        id: "session:session_123",
        href: "/sessions/session_123/transcript",
      }),
    );
  });

  it.each([
    ["chats", "/chats"],
    ["reports", "/reports"],
    ["knowledge indexes", "/knowledge-indexes"],
    ["plugins", "/plugins"],
    ["observers", "/observers"],
    ["payments", "/settings/payments"],
    ["circuit breakers", "/durable/circuit-breakers"],
  ])("finds the %s navigation page", (query, href) => {
    const { result } = renderHook(() => useGlobalSearch(query));

    expect(result.current).toContainEqual(
      expect.objectContaining({ category: "navigation", href }),
    );
  });

  it("hides Payments navigation when machine payments are disabled", () => {
    mockFeatureFlags.machine_payments = false;

    const { result } = renderHook(() => useGlobalSearch("payments"));

    expect(result.current.some((item) => item.href === "/settings/payments")).toBe(false);
  });

  it.each([
    ["skills", "/skills", BookOpen],
    ["capabilities", "/capabilities", Blocks],
    ["plugins", "/plugins", Plug],
  ])("uses the semantic %s icon in navigation search", (query, href, icon) => {
    const { result } = renderHook(() => useGlobalSearch(query));

    expect(result.current).toContainEqual(
      expect.objectContaining({ category: "navigation", href, icon }),
    );
  });

  it.each([
    ["product notes", "memory:mem_product", "/memory/mem_product"],
    ["documentation index", "knowledge-index:kidx_docs", "/knowledge-indexes/kidx_docs"],
    ["email service", "plugin:plugin_email", "/plugins"],
    ["answer quality", "observer:observer_quality", "/observers/observer_quality"],
    ["weekly usage", "report:report_weekly", "/reports"],
  ])("finds the %s entity", (query, id, href) => {
    const { result } = renderHook(() => useGlobalSearch(query));

    expect(result.current).toContainEqual(expect.objectContaining({ id, href }));
  });
});
