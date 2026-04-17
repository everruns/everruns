import { getToolActivitySummaryChip } from "@/components/chat/tool-activity-utils";
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
