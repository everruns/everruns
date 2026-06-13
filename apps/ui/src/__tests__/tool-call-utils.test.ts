import {
  extractMcpAppResources,
  getFullText,
  type McpAppResource,
} from "@/components/chat/tool-call-utils";
import type { ContentPart } from "@/lib/api/types";

const html = "<!doctype html><html><body>Agent card</body></html>";

describe("tool-call-utils MCP App resources", () => {
  it("extracts embedded ui:// HTML resources from MCP content wrappers", () => {
    const result: ContentPart[] = [
      {
        type: "text",
        text: JSON.stringify({
          content: [
            {
              type: "resource",
              uri: "ui://everruns/agent/agent_01/card",
              mime_type: "text/html",
              text: html,
            },
            { type: "text", text: "Agent: customer-support" },
          ],
        }),
      },
    ];

    expect(extractMcpAppResources(result, "agent_get_card")).toEqual<McpAppResource[]>([
      {
        uri: "ui://everruns/agent/agent_01/card",
        mimeType: "text/html",
        html,
      },
    ]);
    expect(getFullText(result)).toBe("Agent: customer-support");
  });

  it("extracts nested MCP resource content shape", () => {
    const result: ContentPart[] = [
      {
        type: "resource",
        uri: "ui://everruns/agent/agent_01/card",
        resource: {
          uri: "ui://everruns/agent/agent_01/card",
          mimeType: "text/html",
          text: html,
        },
      },
    ];

    expect(extractMcpAppResources(result, "agent_get_card")).toEqual([
      {
        uri: "ui://everruns/agent/agent_01/card",
        mimeType: "text/html",
        html,
      },
    ]);
  });

  it("falls back to outer resource fields when nested fields are partial", () => {
    const result: ContentPart[] = [
      {
        type: "resource",
        uri: "ui://everruns/agent/agent_01/card",
        mimeType: "text/html",
        text: html,
        resource: {},
      },
    ];

    expect(extractMcpAppResources(result, "agent_get_card")).toEqual([
      {
        uri: "ui://everruns/agent/agent_01/card",
        mimeType: "text/html",
        html,
      },
    ]);
  });

  it("rejects untrusted ui:// authorities", () => {
    const result: ContentPart[] = [
      {
        type: "text",
        text: JSON.stringify({
          content: [
            {
              type: "resource",
              uri: "ui://evil/agent/agent_01/card",
              mime_type: "text/html",
              text: html,
            },
            {
              type: "resource",
              uri: "ui://everruns.evil.com/agent/agent_01/card",
              mime_type: "text/html",
              text: html,
            },
            {
              type: "resource",
              uri: "ui://everrunsx/agent/agent_01/card",
              mime_type: "text/html",
              text: html,
            },
            {
              type: "resource",
              uri: "ui://everruns",
              mime_type: "text/html",
              text: html,
            },
          ],
        }),
      },
    ];

    expect(extractMcpAppResources(result, "agent_get_card")).toEqual([]);
  });

  it("ignores non-ui and non-html resources", () => {
    const result: ContentPart[] = [
      {
        type: "text",
        text: JSON.stringify({
          content: [
            {
              type: "resource",
              uri: "https://example.com/card.html",
              mime_type: "text/html",
              text: html,
            },
            {
              type: "resource",
              uri: "ui://everruns/agent/agent_01/json",
              mime_type: "application/json",
              text: "{}",
            },
          ],
        }),
      },
    ];

    expect(extractMcpAppResources(result, "agent_get_card")).toEqual([]);
    expect(getFullText(result)).toBe("");
  });

  it("does not parse large non-wrapper text output", () => {
    const text = `${"x".repeat(512 * 1024 + 1)}{"content":[]}`;

    expect(getFullText([{ type: "text", text }])).toBe(text);
    expect(extractMcpAppResources([{ type: "text", text }], "agent_get_card")).toEqual([]);
  });

  it("rejects forged ui://everruns card from a non-get_card tool name", () => {
    const result: ContentPart[] = [
      {
        type: "text",
        text: JSON.stringify({
          content: [
            {
              type: "resource",
              uri: "ui://everruns/agent/agent_01/card",
              mime_type: "text/html",
              text: html,
            },
          ],
        }),
      },
    ];

    const expected: McpAppResource[] = [
      { uri: "ui://everruns/agent/agent_01/card", mimeType: "text/html", html },
    ];

    // First-party and Everruns-prefixed MCP tools are accepted.
    expect(extractMcpAppResources(result, "agent_get_card")).toEqual(expected);
    expect(extractMcpAppResources(result, "mcp_everruns__agent_get_card")).toEqual(expected);

    // Generic remote MCP tool or no tool name — must be rejected.
    expect(extractMcpAppResources(result, "some_remote_tool")).toEqual([]);
    expect(extractMcpAppResources(result, "get_agent")).toEqual([]);
    expect(extractMcpAppResources(result, undefined)).toEqual([]);
    expect(extractMcpAppResources(result)).toEqual([]);
    // Non-everruns MCP server ending in _get_card is rejected.
    expect(extractMcpAppResources(result, "mcp_evil__agent_get_card")).toEqual([]);
    expect(extractMcpAppResources(result, "mcp_other__agent_get_card")).toEqual([]);
  });

  it("rejects ui://everruns URIs that don't match the strict entity/id/card pattern", () => {
    const makeResult = (uri: string): ContentPart[] => [
      {
        type: "resource",
        uri,
        mimeType: "text/html",
        text: html,
      },
    ];

    // Bare prefix only
    expect(extractMcpAppResources(makeResult("ui://everruns/"), "agent_get_card")).toEqual([]);
    // Missing /card suffix
    expect(
      extractMcpAppResources(makeResult("ui://everruns/agent/agent_01"), "agent_get_card"),
    ).toEqual([]);
    // Extra path segments after /card
    expect(
      extractMcpAppResources(
        makeResult("ui://everruns/agent/agent_01/card/extra"),
        "agent_get_card",
      ),
    ).toEqual([]);
    // ID too short (< 2 chars)
    expect(
      extractMcpAppResources(makeResult("ui://everruns/agent/x/card"), "agent_get_card"),
    ).toEqual([]);
    // Query string injection
    expect(
      extractMcpAppResources(makeResult("ui://everruns/agent/agent_01/card?x=1"), "agent_get_card"),
    ).toEqual([]);
  });

  it("rejects HTML exceeding the 64 KiB card size limit", () => {
    const oversized = "x".repeat(64 * 1024 + 1);
    const result: ContentPart[] = [
      {
        type: "resource",
        uri: "ui://everruns/agent/agent_01/card",
        mimeType: "text/html",
        text: oversized,
      },
    ];

    expect(extractMcpAppResources(result, "agent_get_card")).toEqual([]);
  });
});
