import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runInNewContext } from "node:vm";

const html = readFileSync(
  resolve(process.cwd(), "../../crates/server/src/api/mcp_endpoint/apps/workspace.html"),
  "utf8",
);

function shell() {
  document.body.innerHTML = html.split("<body>")[1].split("<script>")[0];
  const parent = { postMessage: jest.fn() };
  let receive!: (event: Partial<MessageEvent>) => Promise<void>;
  runInNewContext(
    html.match(/<script>([\s\S]*)<\/script>/)![1].replace(
      "__WORKSPACE_CONFIG__",
      JSON.stringify({
        origin: "https://app.example.test",
        base: "https://app.example.test/mcp-app",
        kind: "session",
      }),
    ),
    {
      window: {
        parent,
        addEventListener: (_: string, callback: typeof receive) => {
          receive = callback;
        },
      },
      document,
      setTimeout,
      clearTimeout,
    },
  );
  const frame = document.getElementById("workspace") as HTMLIFrameElement;
  // jsdom replaces contentWindow when src changes; the protocol peer stays
  // stable in this harness so messages can be asserted across navigation.
  const child = jest.fn();
  Object.defineProperty(frame, "contentWindow", { value: { postMessage: child } });
  const reply = async (result: unknown) => {
    const request = parent.postMessage.mock.calls.at(-1)![0];
    await receive({
      source: parent as unknown as Window,
      data: { jsonrpc: "2.0", id: request.id, result },
    });
  };
  const host = (method: string, params = {}) =>
    receive({ source: parent as unknown as Window, data: { jsonrpc: "2.0", method, params } });
  return { parent, frame, child, reply, host, receive };
}

beforeEach(() => jest.useFakeTimers());
afterEach(() => {
  jest.clearAllTimers();
  jest.useRealTimers();
  jest.restoreAllMocks();
});

it("clears a successful host request's deadline", async () => {
  const app = shell();
  await app.reply({ hostCapabilities: { serverTools: {} } });
  expect(jest.getTimerCount()).toBe(0);
});

it("lets the user retry initialization after a missing host response", async () => {
  const app = shell();
  await app.host("ui/notifications/tool-result", {
    structuredContent: { view: { kind: "session", id: "session_one" }, organization_id: "org_one" },
  });
  await jest.advanceTimersByTimeAsync(30_000);
  const retry = document.getElementById("retry") as HTMLButtonElement;
  expect(retry).not.toBeNull();
  expect(retry.hidden).toBe(false);
  retry.click();
  await app.reply({ hostCapabilities: { serverTools: {} }, hostContext: { theme: "light" } });
  expect(app.frame.src).toBe("https://app.example.test/mcp-app");
  expect(app.frame.hidden).toBe(false);
  expect(retry.hidden).toBe(true);
});

it("cancels pending work on teardown and ignores late replies", async () => {
  const app = shell();
  await app.reply({ hostCapabilities: { serverTools: {} } });
  await app.host("ui/notifications/tool-result", {
    structuredContent: { view: { kind: "home" }, organization_id: "org_one" },
  });
  await app.receive({
    source: app.frame.contentWindow,
    origin: "https://app.example.test",
    data: { type: "everruns:ready" },
  });
  // Do not await: the host deliberately leaves this write unanswered.
  const write = app.receive({
    source: app.frame.contentWindow,
    origin: "https://app.example.test",
    data: {
      type: "everruns:request",
      id: 5,
      tool: "app_write",
      arguments: { path: "/v1/agents/one", method: "PATCH", body: { description: "new" } },
    },
  });
  const request = app.parent.postMessage.mock.calls.at(-1)![0];
  await app.host("ui/resource-teardown");
  expect(jest.getTimerCount()).toBe(0);
  const count = app.child.mock.calls.length;
  await app.receive({
    source: app.parent as unknown as Window,
    data: { jsonrpc: "2.0", id: request.id, result: { structuredContent: { status: 200 } } },
  });
  await write;
  expect(app.child.mock.calls.length).toBe(count);
});

it("keeps a timed-out write single-shot and lets later reads recover", async () => {
  const app = shell();
  await app.reply({ hostCapabilities: { serverTools: {} } });
  await app.host("ui/notifications/tool-result", {
    structuredContent: { view: { kind: "home" }, organization_id: "org_one" },
  });
  await app.receive({
    source: app.frame.contentWindow,
    origin: "https://app.example.test",
    data: { type: "everruns:ready" },
  });
  const write = app.receive({
    source: app.frame.contentWindow,
    origin: "https://app.example.test",
    data: {
      type: "everruns:request",
      id: 8,
      tool: "app_write",
      arguments: { path: "/v1/agents/one", method: "PATCH" },
    },
  });
  await jest.advanceTimersByTimeAsync(30_000);
  await write;
  expect(app.child).toHaveBeenLastCalledWith(
    { type: "everruns:response", id: 8, error: "Host request timed out" },
    "https://app.example.test",
  );
  const read = app.receive({
    source: app.frame.contentWindow,
    origin: "https://app.example.test",
    data: {
      type: "everruns:request",
      id: 9,
      tool: "app_read",
      arguments: { path: "/v1/agents/one" },
    },
  });
  await app.reply({ structuredContent: { status: 200 } });
  await read;
  expect(app.child).toHaveBeenLastCalledWith(
    { type: "everruns:response", id: 9, result: { status: 200 } },
    "https://app.example.test",
  );
  expect(
    app.parent.postMessage.mock.calls.filter(([message]) => message.params?.name === "app_write"),
  ).toHaveLength(1);
  expect(jest.getTimerCount()).toBe(0);
});

it("does not deliver the previous organization's pending data to the new workspace", async () => {
  const app = shell();
  await app.reply({ hostCapabilities: { serverTools: {} } });
  await app.host("ui/notifications/tool-result", {
    structuredContent: { view: { kind: "home" }, organization_id: "org_one" },
  });
  await app.receive({
    source: app.frame.contentWindow,
    origin: "https://app.example.test",
    data: { type: "everruns:ready" },
  });
  const read = app.receive({
    source: app.frame.contentWindow,
    origin: "https://app.example.test",
    data: { type: "everruns:request", id: 4, tool: "app_read", arguments: { path: "/v1/agents" } },
  });
  const request = app.parent.postMessage.mock.calls.at(-1)![0];
  await app.host("ui/notifications/tool-result", {
    structuredContent: { view: { kind: "home" }, organization_id: "org_two" },
  });
  await read;
  await app.receive({
    source: app.parent as unknown as Window,
    data: {
      jsonrpc: "2.0",
      id: request.id,
      result: { structuredContent: { private: "old organization" } },
    },
  });
  expect(app.child.mock.calls.some(([message]) => message.type === "everruns:response")).toBe(
    false,
  );
  expect(jest.getTimerCount()).toBe(0);
});
