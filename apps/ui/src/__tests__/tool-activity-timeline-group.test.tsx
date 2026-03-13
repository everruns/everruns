import { render, screen } from "@testing-library/react";
import { ToolActivityTimelineGroup } from "@/components/chat/tool-activity-timeline-group";

describe("ToolActivityTimelineGroup", () => {
  it("renders narrated group headline and child rows", () => {
    render(
      <ToolActivityTimelineGroup
        headline="Reading AGENTS.md and searching files"
        completedHeadline="Read AGENTS.md and searched files"
        rows={[
          { id: "tool-1", label: "Read AGENTS.md", state: "completed" },
          { id: "tool-2", label: "Searched files for Doppler", state: "running" },
        ]}
      />,
    );

    expect(screen.getByText("Reading AGENTS.md and searching files")).toBeInTheDocument();
    expect(screen.queryByText("2/2")).not.toBeInTheDocument();
    expect(screen.queryByText("1/2")).not.toBeInTheDocument();
    expect(screen.getByText("Read AGENTS.md")).toBeInTheDocument();
    expect(screen.getByText("Searched files for Doppler")).toBeInTheDocument();
  });
});
