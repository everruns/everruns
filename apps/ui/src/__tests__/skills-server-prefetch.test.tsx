import SkillsPage from "@/app/(main)/skills/page";

const mockServerGet = jest.fn();
const mockServerGetList = jest.fn();
const mockSeedQueryData = jest.fn(
  async (_queryClient: unknown, _queryKey: unknown, load: () => Promise<unknown>) => load(),
);

jest.mock("@tanstack/react-query", () => ({
  HydrationBoundary: ({ children }: { children: React.ReactNode }) => children,
  dehydrate: jest.fn(() => ({})),
}));

jest.mock("@/app/(main)/skills/skills-page-client", () => ({
  __esModule: true,
  default: () => null,
}));

jest.mock("@/lib/server-query", () => ({
  createServerQueryClient: () => ({}),
  getServerRequestContext: async () => ({ apiBaseUrl: "", cookieHeader: "", orgCookieId: null }),
  prefetchAuthBootstrap: async () => ({ currentOrgId: "org_1" }),
  seedQueryData: (queryClient: unknown, queryKey: unknown, load: () => Promise<unknown>) =>
    mockSeedQueryData(queryClient, queryKey, load),
  serverGet: (...args: unknown[]) => mockServerGet(...args),
  serverGetList: (...args: unknown[]) => mockServerGetList(...args),
}));

describe("SkillsPage server prefetch", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockServerGetList.mockResolvedValue([]);
  });

  it("does not fetch or dehydrate skill data when skills are disabled", async () => {
    mockServerGet.mockResolvedValue({ skills: false });

    await SkillsPage();

    expect(mockServerGet).toHaveBeenCalledTimes(1);
    expect(mockServerGet).toHaveBeenCalledWith(expect.anything(), "/v1/orgs/org_1/feature-flags");
    expect(mockServerGetList).not.toHaveBeenCalled();
    expect(mockSeedQueryData).toHaveBeenCalledTimes(1);
  });

  it("preserves skill and policy prefetching when skills are enabled", async () => {
    mockServerGet.mockImplementation(async (_context, endpoint) => {
      if (endpoint === "/v1/orgs/org_1/feature-flags") return { skills: true };
      return {};
    });

    await SkillsPage();

    expect(mockServerGetList).toHaveBeenCalledWith(expect.anything(), "/v1/skills");
    expect(mockServerGet).toHaveBeenCalledWith(expect.anything(), "/v1/skills/config");
    expect(mockSeedQueryData).toHaveBeenCalledTimes(3);
  });
});
