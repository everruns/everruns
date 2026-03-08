import { fireEvent, render, screen } from "@testing-library/react";
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
    expect(screen.queryByLabelText("Tool activity info")).not.toBeInTheDocument();
  });

  it("renders structured bash details with separate stdout and stderr", () => {
    const toolCalls: ToolCallContent[] = [
      {
        id: "tool-shell",
        name: "bash",
        arguments: { command: "cargo test" },
      },
    ];

    const toolResultsMap = new Map<string, ToolCompletedData>([
      [
        "tool-shell",
        {
          tool_call_id: "tool-shell",
          tool_name: "bash",
          success: false,
          status: "error",
          result: [
            {
              type: "text",
              text: JSON.stringify({
                stdout: "running 4 tests\n3 passed",
                stderr: "test suite failed",
                exit_code: 101,
                success: false,
              }),
            },
          ],
        },
      ],
    ]);

    render(<ToolActivityGroup toolCalls={toolCalls} toolResultsMap={toolResultsMap} />);

    expect(screen.getByText("exit 101")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /\$ cargo test exit 101/i }));

    expect(screen.getByText(/running 4 tests\s+3 passed/)).toBeInTheDocument();
    expect(screen.getByText("test suite failed")).toBeInTheDocument();
    expect(screen.queryByText("exit code 101")).not.toBeInTheDocument();
    expect(screen.queryByText(/^output$/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/^stderr$/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/"stdout":/)).not.toBeInTheDocument();
  });

  it("renders bash calls as standalone rows without grouped chrome", () => {
    const toolCalls: ToolCallContent[] = [
      {
        id: "tool-shell",
        name: "bash",
        arguments: { command: "cargo test" },
      },
    ];

    const toolResultsMap = new Map<string, ToolCompletedData>([
      [
        "tool-shell",
        {
          tool_call_id: "tool-shell",
          tool_name: "bash",
          success: false,
          status: "error",
          result: [
            {
              type: "text",
              text: JSON.stringify({
                stdout: "",
                stderr: "test suite failed",
                exit_code: 101,
                success: false,
              }),
            },
          ],
        },
      ],
    ]);

    const { container } = render(
      <ToolActivityGroup toolCalls={toolCalls} toolResultsMap={toolResultsMap} />,
    );

    const root = container.firstElementChild as HTMLElement;
    const shellCard = root.firstElementChild as HTMLElement;

    expect(screen.getByRole("button", { name: /\$ cargo test exit 101/i })).toBeInTheDocument();
    expect(screen.queryByText(/^Shell$/)).not.toBeInTheDocument();
    expect(screen.queryByText("1 of 1 complete")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Tool activity info")).not.toBeInTheDocument();
    expect(shellCard).not.toHaveClass("border");
    expect(shellCard.className).not.toContain("bg-card/95");
    expect(shellCard.className).not.toContain("border-red");
    expect(shellCard.className).not.toContain("bg-red");
  });

  it("keeps bash rows inline while grouping neighboring non-shell activity", () => {
    const toolCalls: ToolCallContent[] = [
      {
        id: "tool-list",
        name: "list_files",
        arguments: { path: "." },
      },
      {
        id: "tool-shell",
        name: "bash",
        arguments: { command: "cargo test" },
      },
      {
        id: "tool-search",
        name: "search_web",
        arguments: { query: "tool activity layout" },
      },
    ];

    const toolResultsMap = new Map<string, ToolCompletedData>([
      [
        "tool-list",
        {
          tool_call_id: "tool-list",
          tool_name: "list_files",
          success: true,
          status: "success",
        },
      ],
      [
        "tool-shell",
        {
          tool_call_id: "tool-shell",
          tool_name: "bash",
          success: true,
          status: "success",
          result: [
            {
              type: "text",
              text: JSON.stringify({
                stdout: "ok",
                stderr: "",
                exit_code: 0,
                success: true,
              }),
            },
          ],
        },
      ],
    ]);

    const { container } = render(
      <ToolActivityGroup toolCalls={toolCalls} toolResultsMap={toolResultsMap} />,
    );

    const text = container.textContent ?? "";
    const listIndex = text.indexOf("List files in current directory");
    const shellIndex = text.indexOf("cargo test");
    const searchIndex = text.indexOf("Search web for tool activity layout");

    expect(listIndex).toBeGreaterThanOrEqual(0);
    expect(shellIndex).toBeGreaterThan(listIndex);
    expect(searchIndex).toBeGreaterThan(shellIndex);
    expect(screen.queryByText(/^Shell$/)).not.toBeInTheDocument();
  });
});
