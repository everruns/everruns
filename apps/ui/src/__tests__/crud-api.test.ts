import { api } from "@/lib/api/client";
import { createCrudApi } from "@/lib/api/crud";

jest.mock("@/lib/api/client", () => ({
  api: {
    post: jest.fn(),
    get: jest.fn(),
    patch: jest.fn(),
    delete: jest.fn(),
  },
}));

const mockApi = jest.mocked(api);

describe("createCrudApi", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("uses the base path for create/get/update/delete/destroy", async () => {
    const crud = createCrudApi<{ id: string; name: string }, { name: string }, { name?: string }>(
      "/v1/widgets",
    );

    mockApi.post.mockResolvedValueOnce({ data: { id: "w1", name: "Widget" } } as never);
    mockApi.get.mockResolvedValueOnce({ data: { id: "w1", name: "Widget" } } as never);
    mockApi.patch.mockResolvedValueOnce({ data: { id: "w1", name: "Renamed" } } as never);
    mockApi.delete.mockResolvedValueOnce(undefined as never);
    mockApi.post.mockResolvedValueOnce(undefined as never);

    await expect(crud.create({ name: "Widget" })).resolves.toEqual({ id: "w1", name: "Widget" });
    await expect(crud.get("w1")).resolves.toEqual({ id: "w1", name: "Widget" });
    await expect(crud.update("w1", { name: "Renamed" })).resolves.toEqual({
      id: "w1",
      name: "Renamed",
    });
    await expect(crud.delete("w1")).resolves.toBeUndefined();
    await expect(crud.destroy("w1")).resolves.toBeUndefined();

    expect(mockApi.post).toHaveBeenNthCalledWith(1, "/v1/widgets", { name: "Widget" });
    expect(mockApi.get).toHaveBeenCalledWith("/v1/widgets/w1");
    expect(mockApi.patch).toHaveBeenCalledWith("/v1/widgets/w1", { name: "Renamed" }, undefined);
    expect(mockApi.delete).toHaveBeenCalledWith("/v1/widgets/w1", undefined);
    expect(mockApi.post).toHaveBeenNthCalledWith(2, "/v1/widgets/w1/delete", undefined, undefined);
  });

  it("sends a change reason as the percent-encoded reason header", async () => {
    const crud = createCrudApi<{ id: string }, never, { name?: string }>("/v1/widgets");
    mockApi.patch.mockResolvedValueOnce({ data: { id: "w1" } } as never);
    mockApi.delete.mockResolvedValueOnce(undefined as never);
    mockApi.post.mockResolvedValueOnce(undefined as never);

    await crud.update("w1", { name: "x" }, "  kid friendly ✓ ");
    await crud.delete("w1", "retired");
    await crud.destroy("w1", "   ");

    expect(mockApi.patch).toHaveBeenCalledWith(
      "/v1/widgets/w1",
      { name: "x" },
      { headers: { "Everruns-Change-Reason": "kid%20friendly%20%E2%9C%93" } },
    );
    expect(mockApi.delete).toHaveBeenCalledWith("/v1/widgets/w1", {
      headers: { "Everruns-Change-Reason": "retired" },
    });
    // A blank reason sends no header: the field never blocks a save.
    expect(mockApi.post).toHaveBeenCalledWith("/v1/widgets/w1/delete", undefined, undefined);
  });

  it("appends include_archived to list requests", async () => {
    const crud = createCrudApi<{ id: string }, never, never>("/v1/widgets");
    mockApi.get.mockResolvedValue({ data: { data: [{ id: "w1" }] } } as never);

    await expect(crud.list()).resolves.toEqual([{ id: "w1" }]);
    await expect(crud.list(true)).resolves.toEqual([{ id: "w1" }]);

    expect(mockApi.get).toHaveBeenNthCalledWith(1, "/v1/widgets");
    expect(mockApi.get).toHaveBeenNthCalledWith(2, "/v1/widgets?include_archived=true");
  });

  it("reuses an existing query string when include_archived is enabled", async () => {
    const crud = createCrudApi<{ id: string }, never, never>("/v1/widgets?limit=20");
    mockApi.get.mockResolvedValue({ data: { data: [] } } as never);

    await crud.list(true);

    expect(mockApi.get).toHaveBeenCalledWith("/v1/widgets?limit=20&include_archived=true");
  });
});
