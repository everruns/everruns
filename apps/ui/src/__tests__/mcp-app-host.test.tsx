import { render } from "@testing-library/react";
import { act } from "react";
import { McpAppFrame, type McpAppHostHandlers } from "@/components/mcp/mcp-app-frame";
import { buildViewCsp, callToolResultFor, readMcpAppCall, withCsp } from "@/lib/mcp-apps";

const APP = {
  server: "sentry",
  tool: "render_chart",
  resource_uri: "ui://sentry/chart.html",
  structured_content: { events: 1284 },
};

describe("readMcpAppCall", () => {
  it("reads a ui:// view from tool details", () => {
    expect(readMcpAppCall({ mcp_app: APP })).toEqual({ ...APP, meta: undefined });
  });

  it("ignores anything that is not a short ui:// view", () => {
    for (const uri of ["https://evil.example/x", "ui://", `ui://${"a".repeat(600)}`]) {
      expect(readMcpAppCall({ mcp_app: { ...APP, resource_uri: uri } })).toBeNull();
    }
    expect(readMcpAppCall(undefined)).toBeNull();
    expect(readMcpAppCall({ mcp_app: { server: "s" } })).toBeNull();
  });
});

describe("view CSP", () => {
  it("allows only declared https origins and blocks the network by default", () => {
    const csp = buildViewCsp({
      connect_domains: ["https://sentry.io", "javascript:alert(1)", "*"],
      resource_domains: [],
      frame_domains: [],
    });
    expect(csp).toContain("default-src 'none'");
    expect(csp).toContain("connect-src https://sentry.io;");
    expect(csp).toContain("frame-src 'none'");
    expect(csp).not.toContain("javascript:");
    expect(buildViewCsp(undefined)).toContain("connect-src 'none'");
  });

  it("puts the CSP before the view's own head content", () => {
    const html = withCsp("<html><head><script>1</script></head></html>", "default-src 'none'");
    expect(html.indexOf("Content-Security-Policy")).toBeLessThan(html.indexOf("<script>"));
    expect(withCsp("<p>x</p>", "a")).toContain('<meta http-equiv="Content-Security-Policy"');
  });
});

describe("callToolResultFor", () => {
  it("rebuilds a CallToolResult with the view-only structured data", () => {
    expect(callToolResultFor([{ type: "text", text: "1,284 events" }], undefined, APP)).toEqual({
      content: [{ type: "text", text: "1,284 events" }],
      isError: false,
      structuredContent: { events: 1284 },
    });
  });
});

function handlers(overrides: Partial<McpAppHostHandlers> = {}): McpAppHostHandlers {
  return {
    openLink: jest.fn(),
    message: jest.fn(),
    requestDisplayMode: jest.fn((mode) => mode),
    ...overrides,
  };
}

function renderFrame(host: McpAppHostHandlers, onReady = jest.fn()) {
  const view = render(
    <McpAppFrame
      html="<html><head></head><body>view</body></html>"
      policy={{ connect_domains: [], resource_domains: [], frame_domains: [] }}
      tool={{ name: "render_chart" }}
      toolCallId="call_1"
      toolInput={{ issue: "X-1" }}
      toolResult={{ content: [], structuredContent: { events: 3 } }}
      displayMode="inline"
      theme="light"
      locale="en"
      handlers={host}
      title="Sentry view"
      onReady={onReady}
    />,
  );
  const iframe = view.container.querySelector("iframe")!;
  const frameWindow = iframe.contentWindow!;
  const posted: unknown[] = [];
  jest.spyOn(frameWindow, "postMessage").mockImplementation((message: unknown) => {
    posted.push(message);
  });
  const send = (data: unknown, source: MessageEventSource = frameWindow) =>
    act(async () => {
      window.dispatchEvent(new MessageEvent("message", { data, source }));
      await Promise.resolve();
    });
  return { iframe, posted, send };
}

describe("McpAppFrame", () => {
  it("runs the view sandboxed with its CSP", () => {
    const { iframe } = renderFrame(handlers());
    expect(iframe.getAttribute("sandbox")).toBe("allow-scripts");
    expect(iframe.getAttribute("srcdoc")).toContain("default-src 'none'");
  });

  it("initializes the view, then sends the call's input and result", async () => {
    const onReady = jest.fn();
    const { posted, send } = renderFrame(handlers(), onReady);
    await send({ jsonrpc: "2.0", id: 1, method: "ui/initialize", params: {} });
    expect(posted[0]).toMatchObject({
      id: 1,
      result: { protocolVersion: "2026-01-26", hostContext: { displayMode: "inline" } },
    });
    // No tool or resource proxying was offered.
    expect(
      (posted[0] as { result: { hostCapabilities: object } }).result.hostCapabilities,
    ).not.toHaveProperty("serverTools");

    await send({ jsonrpc: "2.0", method: "ui/notifications/initialized" });
    expect(posted).toContainEqual({
      jsonrpc: "2.0",
      method: "ui/notifications/tool-input",
      params: { arguments: { issue: "X-1" } },
    });
    expect(posted).toContainEqual(
      expect.objectContaining({
        method: "ui/notifications/tool-result",
        params: { content: [], structuredContent: { events: 3 } },
      }),
    );
    expect(onReady).toHaveBeenCalled();
  });

  it("ignores messages from anywhere but its own frame", async () => {
    const { posted, send } = renderFrame(handlers());
    await send({ jsonrpc: "2.0", id: 1, method: "ui/initialize" }, window);
    expect(posted).toHaveLength(0);
  });

  it("relays tool calls, refuses them when not offered, and makes messages drafts", async () => {
    const callTool = jest.fn().mockResolvedValue({ content: [], isError: false });
    const host = handlers({ callTool });
    const { posted, send } = renderFrame(host);
    await send({
      jsonrpc: "2.0",
      id: 7,
      method: "tools/call",
      params: { name: "get_events", arguments: { range: "24h" } },
    });
    expect(callTool).toHaveBeenCalledWith("get_events", { range: "24h" });
    expect(posted).toContainEqual({
      jsonrpc: "2.0",
      id: 7,
      result: { content: [], isError: false },
    });

    await send({
      jsonrpc: "2.0",
      id: 8,
      method: "ui/message",
      params: { role: "user", content: [{ type: "text", text: "Explain the spike" }] },
    });
    expect(host.message).toHaveBeenCalledWith("Explain the spike");

    await send({ jsonrpc: "2.0", id: 9, method: "ui/open-link", params: { url: "javascript:1" } });
    expect(host.openLink).not.toHaveBeenCalled();

    await send({ jsonrpc: "2.0", id: 10, method: "ui/update-model-context", params: {} });
    expect(posted).toContainEqual(expect.objectContaining({ id: 10, error: expect.anything() }));

    const readOnly = renderFrame(handlers());
    await readOnly.send({ jsonrpc: "2.0", id: 11, method: "tools/call", params: { name: "x" } });
    expect(readOnly.posted).toContainEqual(
      expect.objectContaining({ id: 11, error: expect.objectContaining({ code: -32601 }) }),
    );
  });
});
