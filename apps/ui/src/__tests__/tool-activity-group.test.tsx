import { render, screen } from "@testing-library/react";
import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import type { ToolCompletedData } from "@/lib/api/types";
import type { ToolCallContent } from "@/components/chat/tool-call-utils";

describe("ToolActivityGroup", () => {
  it("groups tool calls into an exploration summary with readable labels", () => {
    const toolCalls: ToolCallContent[] = [
      {
        id: "tool-1",
        name: "list_files",
        arguments: { path: "." },
      },
      {
        id: "tool-2",
        name: "read_file",
        arguments: { path: "README.md" },
      },
      {
        id: "tool-3",
        name: "grep_files",
        arguments: { pattern: "**/README*" },
      },
      {
        id: "tool-4",
        name: "search_web",
        arguments: { query: "project architecture" },
      },
    ];

    const toolResultsMap = new Map<string, ToolCompletedData>([
      [
        "tool-1",
        {
          tool_call_id: "tool-1",
          tool_name: "list_files",
          success: true,
          status: "success",
        },
      ],
    ]);

    render(<ToolActivityGroup toolCalls={toolCalls} toolResultsMap={toolResultsMap} />);

    expect(screen.getByText("Exploring 2 reads, 2 searches")).toBeInTheDocument();
    expect(screen.getByText("List files in current directory")).toBeInTheDocument();
    expect(screen.getByText("Read README.md")).toBeInTheDocument();
    expect(screen.getByText("Find **/README*")).toBeInTheDocument();
    expect(screen.getByText("Search web for project architecture")).toBeInTheDocument();
  });
});
