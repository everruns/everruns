import {
  WorkspaceEvents,
  connectWorkspace,
  workspaceRequest,
  workspaceDownload,
} from "../transport";

describe("MCP workspace events", () => {
  const nativeFetch = global.fetch;
  beforeEach(() => jest.useFakeTimers());
  afterEach(() => {
    global.fetch = nativeFetch;
    jest.useRealTimers();
  });

  it("resumes by sequence and drains busy pages without skipping events", async () => {
    const paths: URL[] = [];
    global.fetch = jest.fn(async (input) => {
      const url = new URL(String(input));
      paths.push(url);
      const from = Number(url.searchParams.get("after_sequence"));
      const end = Math.min(205, from + 100);
      return {
        ok: true,
        json: async () => ({
          data: Array.from({ length: end - from }, (_, index) => ({
            id: `event_${from + index + 1}`,
            sequence: from + index + 1,
            type: "turn.completed",
          })),
        }),
      } as Response;
    });
    const events: number[] = [];
    const stream = new WorkspaceEvents("/api/v1/sessions/session_1/sse?after_sequence=0");
    stream.addEventListener("turn.completed", (event) =>
      events.push(JSON.parse(event.data).sequence),
    );
    await jest.advanceTimersByTimeAsync(10);
    expect(events).toEqual(Array.from({ length: 205 }, (_, i) => i + 1));
    expect(paths.map((url) => url.searchParams.get("after_sequence"))).toEqual(["0", "100", "200"]);
    stream.close();
    await jest.advanceTimersByTimeAsync(5000);
    expect(paths).toHaveLength(3);
  });

  it("resolves a since-id within this session before paging forward", async () => {
    const paths: URL[] = [];
    global.fetch = jest.fn(async (input) => {
      const url = new URL(String(input));
      paths.push(url);
      return {
        ok: true,
        json: async () => ({
          data: url.searchParams.has("around")
            ? [{ id: "event_anchor", sequence: 300 }]
            : [{ id: "event_next", sequence: 301, type: "task.updated" }],
        }),
      } as Response;
    });
    const stream = new WorkspaceEvents("/api/v1/sessions/session_1/sse?since_id=event_anchor");
    const events: string[] = [];
    stream.addEventListener("task.updated", (event) => events.push(JSON.parse(event.data).id));
    await jest.advanceTimersByTimeAsync(0);
    expect(paths[0].searchParams.get("around")).toBe("event_anchor");
    expect(paths[1].searchParams.get("after_sequence")).toBe("300");
    expect(events).toEqual(["event_next"]);
    stream.close();
  });

  it("does not emit a response that arrives after teardown", async () => {
    let finish!: (response: Response) => void;
    global.fetch = jest.fn(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const stream = new WorkspaceEvents("/api/v1/sessions/session_1/sse?after_sequence=0");
    const listener = jest.fn();
    stream.addEventListener("turn.completed", listener);
    await jest.advanceTimersByTimeAsync(0);
    stream.close();
    finish({
      ok: true,
      json: async () => ({ data: [{ id: "late", sequence: 1, type: "turn.completed" }] }),
    } as Response);
    await jest.advanceTimersByTimeAsync(0);
    expect(listener).not.toHaveBeenCalled();
  });
});

it("accepts workspace initialization and replies only from its parent", async () => {
  const initialize = jest.fn();
  const send = jest.spyOn(window.parent, "postMessage").mockImplementation(() => {});
  const disconnect = connectWorkspace(initialize);
  window.dispatchEvent(
    new MessageEvent("message", {
      source: {} as Window,
      data: { type: "everruns:init", view: { kind: "home" }, organization_id: "org_spoofed" },
    }),
  );
  expect(initialize).not.toHaveBeenCalled();
  window.dispatchEvent(
    new MessageEvent("message", {
      source: window.parent,
      data: { type: "everruns:init", view: { kind: "home" }, organization_id: "org_real" },
    }),
  );
  expect(initialize).toHaveBeenCalledTimes(1);
  const request = workspaceRequest("app_read", { path: "/v1/agents" });
  const message = send.mock.calls.at(-1)![0];
  expect(message.arguments.organization_id).toBe("org_real");
  window.dispatchEvent(
    new MessageEvent("message", {
      source: window.parent,
      data: { type: "everruns:response", id: message.id, result: { status: 200 } },
    }),
  );
  await expect(request).resolves.toEqual({ status: 200 });
  disconnect();
  send.mockRestore();
});

it("hands exact binary bytes to the host and surfaces a rejected save", async () => {
  const send = jest.spyOn(window.parent, "postMessage").mockImplementation(() => {});
  const disconnect = connectWorkspace(() => {});
  try {
    const download = workspaceDownload(
      "résumé.bin",
      new Blob([new Uint8Array([0, 255, 128, 65])], { type: "application/octet-stream" }),
    );
    // FileReader completes asynchronously before the host request is emitted.
    await new Promise((resolve) => setTimeout(resolve, 20));
    const message = send.mock.calls.at(-1)![0];
    expect(message).toMatchObject({
      type: "everruns:download",
      filename: "résumé.bin",
      mimeType: "application/octet-stream",
      blob: "AP+AQQ==",
    });
    window.dispatchEvent(
      new MessageEvent("message", {
        source: window.parent,
        data: { type: "everruns:response", id: message.id, error: "Save declined" },
      }),
    );
    await expect(download).rejects.toThrow("Save declined");
  } finally {
    disconnect();
    send.mockRestore();
  }
});

it("rejects oversized downloads before crossing the host bridge", async () => {
  await expect(
    workspaceDownload("large.bin", new Blob([new Uint8Array(8 * 1024 * 1024 + 1)])),
  ).rejects.toThrow("8 MiB");
});
