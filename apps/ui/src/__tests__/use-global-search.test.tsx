import { act, renderHook } from "@testing-library/react";
import {
  AgentIcon,
  SessionIcon,
  HarnessDomainIcon,
  KnowledgeIcon,
  SkillsIcon,
} from "@/components/icons/facet-icons";
import { useGlobalSearch } from "@/hooks/use-global-search";
import { settingsNavigationSections } from "@/lib/settings-navigation";
import { defaultNavigationSections } from "@/lib/navigation";
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
jest.mock("@/hooks/use-virtual-users", () => ({
  useVirtualUsers: () => ({ data: [] }),
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
const mockUseKnowledgeCollections = jest.fn((_options?: { enabled?: boolean }) => ({
  data: {
    pages: [
      {
        data: [
          {
            id: "kn_docs",
            name: "Documentation Collection",
            description: "Published documentation",
          },
        ],
      },
    ],
  },
}));
jest.mock("@/components/knowledge/use-knowledge", () => ({
  useKnowledgeCollections: (options?: { enabled?: boolean }) =>
    mockUseKnowledgeCollections(options),
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

const mockFeatureFlags = {
  evals: false,
  extractions: true,
  skills: true,
  observers: true,
};
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlags: () => mockFeatureFlags,
}));

const mockSetCurrentOrg = jest.fn();
const mockHasRole = jest.fn(() => true);
const mockDurableAllowed = { value: true };
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => mockDurableAllowed.value }),
}));

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
    hasRole: mockHasRole,
  }),
}));

describe("useGlobalSearch", () => {
  beforeEach(() => {
    mockSetCurrentOrg.mockClear();
    mockHasRole.mockReturnValue(true);
    mockDurableAllowed.value = true;
    mockUseAgents.mockClear();
    mockUseSessions.mockReset();
    mockUseSessions.mockReturnValue({ data: { data: [] } });
    mockUseSkills.mockClear();
    mockUseEvals.mockClear();
    mockUseMemories.mockClear();
    mockUseKnowledgeCollections.mockClear();
    mockUseInstalledPlugins.mockClear();
    Object.assign(mockFeatureFlags, {
      evals: false,
      extractions: true,
      skills: true,
      observers: true,
    });
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
    });

    renderHook(() => useGlobalSearch("memory"));

    expect(mockUseSkills).toHaveBeenCalledWith({ enabled: false });
    expect(mockUseEvals).toHaveBeenCalledWith({ enabled: false });
    expect(mockUseKnowledgeCollections).toHaveBeenCalledWith({ enabled: true });
    expect(mockUseInstalledPlugins).toHaveBeenCalledWith({ enabled: true });
  });

  it("finds memory without a feature flag", () => {
    const { result } = renderHook(() => useGlobalSearch("memory"));

    expect(mockUseMemories).toHaveBeenCalledWith({ enabled: true });
    expect(result.current).toContainEqual(expect.objectContaining({ href: "/memory" }));
  });

  it("does not enable entity fetches before the user enters a query", () => {
    renderHook(() => useGlobalSearch(""));

    expect(mockUseAgents).toHaveBeenCalledWith({ enabled: false });
    expect(mockUseKnowledgeCollections).toHaveBeenCalledWith({ enabled: false });
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

    expect(
      result.current.some(
        (item) => item.category === "navigation" && item.href === "/settings/organization",
      ),
    ).toBe(true);
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
        href: "/sessions/session_123/trace",
      }),
    );
  });

  it.each([
    ["chats", "/chats"],
    ["playground", "/playground"],
    ["sandbox templates", "/sandboxes"],
    ["slack", "/settings/slack"],
    ["health", "/settings/health"],
    ["new side chat", "/chats/new"],
    ["all chats", "/chats/history"],
    ["knowledge", "/knowledge"],
    ["plugins", "/settings/plugins"],
    ["observers", "/observers"],
    ["circuit breakers", "/durable/circuit-breakers"],
  ])("finds the %s navigation page", (query, href) => {
    const { result } = renderHook(() => useGlobalSearch(query));

    expect(result.current).toContainEqual(
      expect.objectContaining({ category: "navigation", href }),
    );
  });

  it("offers every available sidebar destination before typing", () => {
    const { result } = renderHook(() => useGlobalSearch(""));
    for (const section of defaultNavigationSections.filter((section) => !section.devOnly)) {
      for (const item of section.items) {
        if (item.flag && !mockFeatureFlags[item.flag as keyof typeof mockFeatureFlags]) continue;
        expect(result.current).toContainEqual(
          expect.objectContaining({
            category: "navigation",
            href: item.href,
            title: item.name,
            icon: item.icon,
          }),
        );
      }
    }
  });

  it("offers every available Settings destination with its current name and icon", () => {
    const { result } = renderHook(() => useGlobalSearch("settings"));
    for (const section of settingsNavigationSections) {
      for (const item of section.items) {
        if (item.flag && !mockFeatureFlags[item.flag as keyof typeof mockFeatureFlags]) continue;
        // The organization page also serves as the shell's Settings landing destination.
        const expected =
          item.href === "/settings/organization"
            ? { category: "navigation", href: item.href, title: "Settings" }
            : { category: "navigation", href: item.href, title: item.name, icon: item.icon };
        expect(result.current).toContainEqual(expect.objectContaining(expected));
      }
    }
  });

  it("hides admin-only, denied durable, and development navigation", () => {
    mockHasRole.mockReturnValue(false);
    mockDurableAllowed.value = false;
    const { result } = renderHook(() => useGlobalSearch(""));
    expect(result.current.some((item) => item.href === "/approvals")).toBe(false);
    expect(result.current.some((item) => item.href.startsWith("/durable"))).toBe(false);
    expect(result.current.some((item) => item.href === "/dev")).toBe(false);
  });

  it("offers MCP catalog only to people who can manage it", () => {
    const allowed = renderHook(() => useGlobalSearch("mcp catalog"));
    expect(allowed.result.current).toContainEqual(
      expect.objectContaining({ category: "navigation", href: "/settings/mcp-catalog" }),
    );
    expect(allowed.result.current.some((item) => item.href === "/mcp-servers")).toBe(false);

    mockDurableAllowed.value = false;
    const denied = renderHook(() => useGlobalSearch("mcp catalog"));
    expect(
      denied.result.current.some(
        (item) => item.category === "navigation" && item.href === "/settings/mcp-catalog",
      ),
    ).toBe(false);
  });

  it.each([
    { adopted: true, canManageModels: true, visible: true },
    { adopted: true, canManageModels: false, visible: false },
    { adopted: false, canManageModels: true, visible: false },
  ])("filters Extractions with adoption=$adopted and model management=$canManageModels", (test) => {
    mockFeatureFlags.extractions = test.adopted;
    mockDurableAllowed.value = test.canManageModels;
    const { result } = renderHook(() => useGlobalSearch("extractions"));
    expect(
      result.current.some((item) => item.category === "navigation" && item.href === "/extractions"),
    ).toBe(test.visible);
  });

  it("does not return the retired Apps route", () => {
    const { result } = renderHook(() => useGlobalSearch("apps"));

    expect(result.current.some((item) => item.href.startsWith("/apps"))).toBe(false);
  });

  it("searches for Knowledge without treating the word as a collection ID", () => {
    const { result } = renderHook(() => useGlobalSearch("knowledge"));
    expect(result.current.some((item) => item.category === "id")).toBe(false);
  });

  it.each([
    ["agent_unknown", "/agents/agent_unknown", AgentIcon],
    ["session_unknown", "/sessions/session_unknown", SessionIcon],
    ["harness_unknown", "/harnesses/harness_unknown", HarnessDomainIcon],
    ["skill_unknown", "/skills/skill_unknown", SkillsIcon],
    ["kn_unknown", "/knowledge/kn_unknown", KnowledgeIcon],
  ])("uses the entity's own domain glyph for an ID lookup: %s", (query, href, icon) => {
    const { result } = renderHook(() => useGlobalSearch(query));
    expect(result.current).toContainEqual(expect.objectContaining({ category: "id", href, icon }));
  });

  it.each([
    ["product notes", "memory:mem_product", "/memory/mem_product"],
    ["documentation collection", "knowledge:kn_docs", "/knowledge/kn_docs"],
    ["email service", "plugin:plugin_email", "/settings/plugins"],
    ["answer quality", "observer:observer_quality", "/observers/observer_quality"],
  ])("finds the %s entity", (query, id, href) => {
    const { result } = renderHook(() => useGlobalSearch(query));

    expect(result.current).toContainEqual(expect.objectContaining({ id, href }));
  });
});
