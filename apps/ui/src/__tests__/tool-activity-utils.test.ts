import {
  formatResultDetails,
  getResultPreview,
  getToolActivitySummaryChip,
} from "@/components/chat/tool-activity-utils";
import type { ToolCompletedData } from "@/lib/api/types";
import type { ToolCallContent } from "@/components/chat/tool-call-utils";

function textResult(toolName: string, text: string): ToolCompletedData {
  return {
    tool_call_id: "tool-1",
    tool_name: toolName,
    success: true,
    status: "success",
    result: [{ type: "text", text }],
  };
}

describe("getToolActivitySummaryChip", () => {
  it("renders scheduled monitor creation with a compact cadence", () => {
    const toolCall: ToolCallContent = {
      id: "tool-1",
      name: "spawn_background",
      arguments: {},
    };

    const summary = getToolActivitySummaryChip(
      toolCall,
      textResult(
        "spawn_background",
        JSON.stringify({
          status: "scheduled",
          title: "Watch PR 1319",
          cron_expression: "*/10 * * * *",
          timezone: "America/Chicago",
        }),
      ),
    );

    expect(summary).toEqual({
      status: "Created",
      title: "Watch PR 1319",
      schedule: "Every 10m",
    });
  });

  it("renders cancelled monitor chips with the extracted monitor title", () => {
    const toolCall: ToolCallContent = {
      id: "tool-2",
      name: "cancel_schedule",
      arguments: {},
    };

    const summary = getToolActivitySummaryChip(
      toolCall,
      textResult(
        "cancel_schedule",
        JSON.stringify({
          cancelled: true,
          description: `Monitor: Watch PR 1319

This scheduled monitor fired. Start the background run now.

spawn_background payload:
{"tool":"github","title":"Watch PR 1319","signal_on_completion":true,"args":{}}`,
        }),
      ),
    );

    expect(summary).toEqual({
      status: "Deleted",
      title: "Watch PR 1319",
    });
  });
});

describe("structured tool result rendering", () => {
  it("unwraps quoted platform JSON and summarizes collections", () => {
    const discover = { id: "tool-1", name: "discover", arguments: {} };
    const quotedJson = JSON.stringify(
      JSON.stringify({ count: 2, operations: [{ name: "list_agents" }, { name: "get_agent" }] }),
    );

    expect(getResultPreview(discover, textResult("discover", quotedJson))).toBe("2 operations");
    expect(formatResultDetails(discover, quotedJson)).toContain("count: 2");
    expect(formatResultDetails(discover, quotedJson)).not.toContain('\\"count\\"');
  });

  it("summarizes structured output inside a distillation envelope", () => {
    const discover = { id: "tool-1", name: "discover", arguments: {} };
    const distilled = JSON.stringify({
      distilled: true,
      distill_note: "Machine transport note",
      distilled_output: JSON.stringify({ count: 49, operations: [{ name: "list_agents" }] }),
    });

    expect(getResultPreview(discover, textResult("discover", distilled))).toBe("49 operations");
  });

  it("uses a human fallback when a distilled JSON sample is clipped", () => {
    const discover = { id: "tool-1", name: "discover", arguments: {} };
    const distilled = JSON.stringify({
      distilled: true,
      distill_note: "Machine transport note",
      distilled_output: '{"count":49,"operations":[…',
    });

    expect(getResultPreview(discover, textResult("discover", distilled))).toBe(
      "Large result available in Details",
    );
  });

  it("uses a human noun for paginated data", () => {
    const query = { id: "tool-1", name: "query", arguments: {} };
    const result = JSON.stringify({ data: [{}], total: 1 });

    expect(getResultPreview(query, textResult("query", result))).toBe("1 result");
  });

  it("keeps plain text useful and does not preview unsummarized JSON", () => {
    const web = { id: "tool-1", name: "web_fetch", arguments: {} };

    expect(getResultPreview(web, textResult("web_fetch", "Fetched documentation"))).toBe(
      "Fetched documentation",
    );
    expect(
      getResultPreview(web, textResult("web_fetch", JSON.stringify({ opaque: { nested: true } }))),
    ).toBeNull();
  });

  it("masks secret-like fields recursively in details", () => {
    const plugin = { id: "tool-1", name: "plugin_call", arguments: {} };
    const details = formatResultDetails(
      plugin,
      JSON.stringify({
        result: {
          api_key: "sk-live-secret",
          access_token: "token-secret",
          bearerToken: "bearer-secret",
        },
      }),
    );

    expect(details).not.toContain("sk-live-secret");
    expect(details).not.toContain("token-secret");
    expect(details).not.toContain("bearer-secret");
    expect(details).toContain("[hidden]");
  });
});
