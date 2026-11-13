import { listCapabilities } from "@/lib/api/capabilities";
import { getActiveOrgId, ORG_HEADER, setActiveOrgId } from "@/lib/api/active-org";
import { api } from "@/lib/api/client";

jest.mock("@/lib/api/client", () => ({ api: { get: jest.fn() } }));

const mockGet = jest.mocked(api.get);
const firstPage = Array.from({ length: 100 }, (_, index) => ({ id: `builtin-${index}` }));
const plugin = { id: "plugin:installed" };

describe("listCapabilities", () => {
  beforeEach(() => {
    jest.resetAllMocks();
    setActiveOrgId("org-a");
  });

  afterEach(() => setActiveOrgId(null));

  it("includes installed capabilities beyond the first 100 results", async () => {
    mockGet
      .mockResolvedValueOnce({ data: { data: firstPage, total: 101, offset: 0, limit: 100 } })
      .mockResolvedValueOnce({ data: { data: [plugin], total: 101, offset: 100, limit: 100 } });

    const capabilities = await listCapabilities();

    expect(capabilities).toEqual([...firstPage, plugin]);
    expect(mockGet).toHaveBeenLastCalledWith("/v1/capabilities?offset=100&limit=100", {
      headers: { [ORG_HEADER]: "org-a" },
    });
  });

  it("keeps retired visibility and the original org on every page during an org switch", async () => {
    mockGet
      .mockImplementationOnce(async () => {
        setActiveOrgId("org-b");
        return { data: { data: firstPage, total: 101, offset: 0, limit: 100 } };
      })
      .mockResolvedValueOnce({ data: { data: [plugin], total: 101, offset: 100, limit: 100 } });

    expect(await listCapabilities(true)).toEqual([...firstPage, plugin]);
    expect(getActiveOrgId()).toBe("org-b");
    expect(mockGet).toHaveBeenNthCalledWith(
      1,
      "/v1/capabilities?offset=0&limit=100&include_retired=true",
      { headers: { [ORG_HEADER]: "org-a" } },
    );
    expect(mockGet).toHaveBeenNthCalledWith(
      2,
      "/v1/capabilities?offset=100&limit=100&include_retired=true",
      { headers: { [ORG_HEADER]: "org-a" } },
    );
  });

  it("rejects a failed later page instead of returning a partial catalog", async () => {
    mockGet
      .mockResolvedValueOnce({ data: { data: firstPage, total: 101, offset: 0, limit: 100 } })
      .mockRejectedValueOnce(new Error("page failed"));

    await expect(listCapabilities()).rejects.toThrow("page failed");
  });

  it("honors the query's org when a retry starts after switching orgs", async () => {
    setActiveOrgId("org-b");
    mockGet.mockResolvedValueOnce({ data: { data: [], total: 0, offset: 0, limit: 100 } });

    expect(await listCapabilities(false, "org-a")).toEqual([]);
    expect(mockGet).toHaveBeenCalledWith("/v1/capabilities?offset=0&limit=100", {
      headers: { [ORG_HEADER]: "org-a" },
    });
  });
});
