import { fireEvent, render, screen } from "@testing-library/react";
import { ToolGroupNode } from "@/components/trajectory/trajectory-nodes";

jest.mock("@xyflow/react", () => ({
  Handle: () => null,
  Position: { Top: "top", Bottom: "bottom" },
}));

describe("ToolGroupNode", () => {
  it("shows action, status, duration, and expandable redacted details", () => {
    render(
      <ToolGroupNode
        id="tools"
        type="toolGroup"
        selected={false}
        dragging={false}
        draggable={false}
        selectable={false}
        deletable={false}
        zIndex={0}
        isConnectable={false}
        positionAbsoluteX={0}
        positionAbsoluteY={0}
        data={{
          label: "Tools (1)",
          tools: [
            {
              id: "call-1",
              name: "search_web",
              displayName: "Web search",
              label: "Searched the release notes",
              success: true,
              status: "success",
              durationMs: 1250,
              arguments: { query: "release notes", api_key: "do-not-render" },
              resultText: JSON.stringify({ matches: 3, token: "also-hidden" }),
            },
          ],
          successCount: 1,
          errorCount: 0,
          durationMs: 1250,
          eventId: "event-1",
          timestamp: "2026-08-06T12:00:00Z",
        }}
      />,
    );

    expect(screen.getByText("Searched the release notes")).toBeInTheDocument();
    expect(screen.getByText("search_web")).toBeInTheDocument();
    expect(screen.getByText("success")).toBeInTheDocument();
    expect(screen.getAllByText("1.3s").length).toBeGreaterThan(0);

    fireEvent.click(screen.getByText("Searched the release notes"));
    expect(screen.getByText("Input")).toBeInTheDocument();
    expect(screen.getByText("Result")).toBeInTheDocument();
    expect(screen.getAllByText(/\[redacted\]/)).toHaveLength(2);
    expect(screen.queryByText(/do-not-render|also-hidden/)).not.toBeInTheDocument();
  });

  it("renders pending calls as running rather than successful", () => {
    render(
      <ToolGroupNode
        id="tools"
        type="toolGroup"
        selected={false}
        dragging={false}
        draggable={false}
        selectable={false}
        deletable={false}
        zIndex={0}
        isConnectable={false}
        positionAbsoluteX={0}
        positionAbsoluteY={0}
        data={{
          label: "Tools (1)",
          tools: [
            {
              id: "call-1",
              name: "bash",
              label: "Run tests",
              status: "pending",
              arguments: { command: "pnpm test" },
            },
          ],
          successCount: 0,
          errorCount: 0,
          eventId: "event-1",
          timestamp: "2026-08-06T12:00:00Z",
        }}
      />,
    );

    expect(screen.getByText("running")).toBeInTheDocument();
    expect(screen.queryByText("success")).not.toBeInTheDocument();
  });

  it("redacts secret assignments in plain-text results", () => {
    render(
      <ToolGroupNode
        id="tools"
        type="toolGroup"
        selected={false}
        dragging={false}
        draggable={false}
        selectable={false}
        deletable={false}
        zIndex={0}
        isConnectable={false}
        positionAbsoluteX={0}
        positionAbsoluteY={0}
        data={{
          label: "Tools (1)",
          tools: [
            {
              id: "call-1",
              name: "bash",
              label: "Ran command",
              status: "success",
              success: true,
              resultText: "OPENAI_API_KEY=sk-live-secret\nok",
            },
          ],
          successCount: 1,
          errorCount: 0,
          eventId: "event-1",
          timestamp: "2026-08-06T12:00:00Z",
        }}
      />,
    );

    fireEvent.click(screen.getByText("Ran command"));
    expect(screen.getByText(/OPENAI_API_KEY=\[redacted\]/)).toBeInTheDocument();
    expect(screen.queryByText(/sk-live-secret/)).not.toBeInTheDocument();
  });
});
