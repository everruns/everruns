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

    expect(extractMcpAppResources(result)).toEqual<McpAppResource[]>([
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

    expect(extractMcpAppResources(result)).toEqual([
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

    expect(extractMcpAppResources(result)).toEqual([
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

    expect(extractMcpAppResources(result)).toEqual([]);
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

    expect(extractMcpAppResources(result)).toEqual([]);
    expect(getFullText(result)).toBe("");
  });

  it("does not parse large non-wrapper text output", () => {
    const text = `${"x".repeat(512 * 1024 + 1)}{"content":[]}`;

    expect(getFullText([{ type: "text", text }])).toBe(text);
    expect(extractMcpAppResources([{ type: "text", text }])).toEqual([]);
  });
});
