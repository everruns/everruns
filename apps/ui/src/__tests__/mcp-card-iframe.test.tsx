import { render } from "@testing-library/react";
import { act } from "react";
import { McpCardIframe } from "@/components/mcp/mcp-card-iframe";

const SAMPLE_HTML = "<!doctype html><html><body>x</body></html>";
const URI = "ui://everruns/agent/agent_01/card";

describe("McpCardIframe", () => {
  it("renders a sandboxed iframe with srcDoc and no allow-same-origin", () => {
    const { container } = render(<McpCardIframe uri={URI} html={SAMPLE_HTML} />);
    const iframe = container.querySelector("iframe")!;
    expect(iframe.getAttribute("sandbox")).toBe("allow-scripts");
    expect(iframe.getAttribute("referrerpolicy")).toBe("no-referrer");
    expect(iframe.getAttribute("srcdoc")).toBe(SAMPLE_HTML);
    expect(iframe.getAttribute("data-mcp-card-uri")).toBe(URI);
    expect(iframe.title).toContain(URI);
  });

  it("ignores postMessage events whose source is not the iframe contentWindow", () => {
    const onAction = jest.fn();
    render(<McpCardIframe uri={URI} html={SAMPLE_HTML} onAction={onAction} />);

    act(() => {
      window.dispatchEvent(
        new MessageEvent("message", {
          data: { type: "tool", payload: { toolName: "agent_run", params: {} } },
          source: window, // not the iframe.contentWindow
        }),
      );
    });

    expect(onAction).not.toHaveBeenCalled();
  });

  it("ignores messages whose shape is not a known card message", () => {
    const onAction = jest.fn();
    const { container } = render(
      <McpCardIframe uri={URI} html={SAMPLE_HTML} onAction={onAction} />,
    );
    const iframe = container.querySelector("iframe")!;
    const source = iframe.contentWindow as Window;

    act(() => {
      window.dispatchEvent(new MessageEvent("message", { data: { type: "tool" }, source }));
      window.dispatchEvent(
        new MessageEvent("message", { data: { type: "unknown", payload: {} }, source }),
      );
      window.dispatchEvent(new MessageEvent("message", { data: "string-not-object", source }));
    });

    expect(onAction).not.toHaveBeenCalled();
  });

  it("rejects per-type payload shapes that don't match the CardMessage variant", () => {
    const onAction = jest.fn();
    const { container } = render(
      <McpCardIframe uri={URI} html={SAMPLE_HTML} onAction={onAction} />,
    );
    const iframe = container.querySelector("iframe")!;
    const source = iframe.contentWindow as Window;

    const bad = [
      // Arrays must be rejected even though typeof is "object".
      [1, 2, 3],
      // tool: missing toolName
      { type: "tool", payload: { params: {} } },
      // tool: toolName is empty string
      { type: "tool", payload: { toolName: "" } },
      // tool: params is not an object
      { type: "tool", payload: { toolName: "x", params: "not-an-object" } },
      // prompt: missing prompt
      { type: "prompt", payload: {} },
      // intent: missing intent
      { type: "intent", payload: { params: {} } },
      // intent: params is array
      { type: "intent", payload: { intent: "open_agent", params: [1] } },
      // link: missing url
      { type: "link", payload: {} },
      // notify: missing message
      { type: "notify", payload: {} },
      // payload itself is an array
      { type: "notify", payload: [] },
    ];

    act(() => {
      for (const data of bad) {
        window.dispatchEvent(new MessageEvent("message", { data, source }));
      }
    });

    expect(onAction).not.toHaveBeenCalled();
  });

  it("forwards a well-formed message from the iframe to onAction", () => {
    const onAction = jest.fn();
    const { container } = render(
      <McpCardIframe uri={URI} html={SAMPLE_HTML} onAction={onAction} />,
    );
    const iframe = container.querySelector("iframe")!;
    const source = iframe.contentWindow as Window;

    act(() => {
      window.dispatchEvent(
        new MessageEvent("message", {
          data: {
            type: "tool",
            payload: { toolName: "agent_run", params: { id: "x" } },
          },
          source,
        }),
      );
    });

    expect(onAction).toHaveBeenCalledTimes(1);
    expect(onAction).toHaveBeenCalledWith({
      type: "tool",
      payload: { toolName: "agent_run", params: { id: "x" } },
    });
  });

  it("rate-limits a flood of messages from the iframe", () => {
    const onAction = jest.fn();
    const { container } = render(
      <McpCardIframe uri={URI} html={SAMPLE_HTML} onAction={onAction} />,
    );
    const iframe = container.querySelector("iframe")!;
    const source = iframe.contentWindow as Window;

    act(() => {
      for (let i = 0; i < 50; i++) {
        window.dispatchEvent(
          new MessageEvent("message", {
            data: { type: "notify", payload: { message: `msg-${i}` } },
            source,
          }),
        );
      }
    });

    // Bucket starts at 10 tokens; 40 of 50 flood messages should be dropped.
    expect(onAction.mock.calls.length).toBeLessThanOrEqual(10);
    expect(onAction.mock.calls.length).toBeGreaterThan(0);
  });
});
