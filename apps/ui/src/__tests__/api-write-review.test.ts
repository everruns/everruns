import { api } from "@/lib/api/client";
import { setActiveOrgId } from "@/lib/api/active-org";
import { observeResourceWrites, type ObservedWrite } from "@/lib/api/write-observer";

let stop: (() => void) | undefined;
const committed = { kind: "model", id: "model_01933b5a000070008000000000000001", revision: 2 };
const endpoint = `/v1/models/${committed.id}`;
function response(status: number, value: unknown = { id: committed.id }) {
  return {
    ok: status < 400,
    status,
    statusText: "",
    headers: {
      get: (name: string) =>
        name.toLowerCase() === "everruns-committed-change" ? JSON.stringify(committed) : null,
    },
    json: async () => value,
    text: async () => JSON.stringify(value),
  };
}
beforeEach(() => {
  global.fetch = jest.fn();
  setActiveOrgId("org_original");
});
afterEach(() => {
  stop?.();
  stop = undefined;
  setActiveOrgId(null);
});

it("correlates a successful save with server metadata without exposing its request body to the observer", async () => {
  const succeeded = jest.fn(async () => {}),
    failed = jest.fn();
  const observer = jest.fn(async (): Promise<ObservedWrite> => ({
    headers: { "Everruns-Expected-Change": "guard" },
    succeeded,
    failed,
  }));
  stop = observeResourceWrites(observer);
  (fetch as jest.Mock).mockResolvedValue(response(200));
  await api.patch(endpoint, { display_name: "Reviewed", manual_secret: "fixture-only" });
  expect(observer).toHaveBeenCalledWith("PATCH", endpoint);
  expect(observer.mock.calls[0]).toHaveLength(2);
  expect(succeeded).toHaveBeenCalledWith(committed);
  expect(failed).not.toHaveBeenCalled();
  expect((fetch as jest.Mock).mock.calls[0][1].headers["Everruns-Expected-Change"]).toBe("guard");
});

it("does not reject a committed mutation or repeat it if loading its review fails", async () => {
  const failed = jest.fn();
  stop = observeResourceWrites(async () => ({
    succeeded: async () => {
      throw new Error("history offline");
    },
    failed,
  }));
  (fetch as jest.Mock).mockResolvedValue(response(200));
  await expect(api.patch(endpoint, { display_name: "Reviewed" })).resolves.toEqual({
    data: { id: committed.id },
  });
  expect(fetch).toHaveBeenCalledTimes(1);
  expect(failed).toHaveBeenCalledTimes(1);
});

it("reuses one review guard across authentication refresh and reports server conflicts as failed writes", async () => {
  const succeeded = jest.fn(async () => {}),
    failed = jest.fn();
  const observer = jest.fn(async (): Promise<ObservedWrite> => ({ succeeded, failed }));
  stop = observeResourceWrites(observer);
  (fetch as jest.Mock)
    .mockResolvedValueOnce(response(401))
    .mockResolvedValueOnce(response(200))
    .mockResolvedValueOnce(response(409, { detail: "The saved version changed" }));
  await expect(api.patch(endpoint, { display_name: "Reviewed" })).rejects.toMatchObject({
    status: 409,
  });
  expect(observer).toHaveBeenCalledTimes(1);
  expect(succeeded).not.toHaveBeenCalled();
  expect(failed).toHaveBeenCalledTimes(1);
});

it("captures tenant headers before an asynchronous pre-save check", async () => {
  let resolve!: (value: ObservedWrite) => void;
  stop = observeResourceWrites(
    () =>
      new Promise<ObservedWrite>((ready) => {
        resolve = ready;
      }),
  );
  (fetch as jest.Mock).mockResolvedValue(response(200));
  const pending = api.patch(endpoint, { display_name: "Reviewed" });
  setActiveOrgId("org_other");
  resolve({ succeeded: async () => {}, failed: () => {} });
  await pending;
  expect((fetch as jest.Mock).mock.calls[0][1].headers["X-Org-Id"]).toBe("org_original");
});

it("keeps private query parameters outside the write observer", async () => {
  const observer = jest.fn(async () => undefined);
  stop = observeResourceWrites(observer);
  (fetch as jest.Mock).mockResolvedValue(response(200));
  await api.get("/v1/search?query=browser-only-private-fixture");
  expect(observer).toHaveBeenCalledWith("GET", "/v1/search");
  expect((fetch as jest.Mock).mock.calls[0][0]).toContain("browser-only-private-fixture");
});
it("forwards a history-read abort signal without treating it as a mutation retry", async () => {
  const controller = new AbortController();
  (fetch as jest.Mock).mockResolvedValue(response(200));
  await api.get("/v1/history/fixture", { signal: controller.signal });
  expect((fetch as jest.Mock).mock.calls[0][1].signal).toBe(controller.signal);
  expect(fetch).toHaveBeenCalledTimes(1);
});
