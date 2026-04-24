import { render, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import { ToolActivityTimelineGroup } from "@/components/chat/tool-activity-timeline-group";
import { LocaleProvider } from "@/providers/locale-provider";

function renderWithLocale(ui: ReactElement) {
  return render(<LocaleProvider>{ui}</LocaleProvider>);
}

describe("ToolActivityTimelineGroup", () => {
  it("renders single row inline without duplicate headline", () => {
    renderWithLocale(
      <ToolActivityTimelineGroup
        headline="Ran List Capabilities"
        completedHeadline="Ran List Capabilities"
        rows={[{ id: "tool-1", label: "List Capabilities", state: "completed" }]}
      />,
    );

    // Headline should appear exactly once (as the row label)
    const matches = screen.getAllByText("Ran List Capabilities");
    expect(matches).toHaveLength(1);
  });

  it("renders narrated group headline and child rows", () => {
    renderWithLocale(
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

  it("redacts secret_store values in preview and details", () => {
    renderWithLocale(
      <ToolActivityTimelineGroup
        headline="Reading secret"
        rows={[
          {
            id: "tool-1",
            label: "Read secret",
            state: "completed",
            result: {
              tool_call_id: "tool-1",
              tool_name: "secret_store",
              success: true,
              status: "success",
              result: [
                {
                  type: "text",
                  text: JSON.stringify({
                    operation: "get",
                    name: "OPENAI_API_KEY",
                    found: true,
                    value: "sk-live-secret",
                  }),
                },
              ],
            },
          },
        ]}
      />,
    );

    expect(screen.getByText("OPENAI_API_KEY found")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /details/i })).toBeInTheDocument();
    expect(screen.queryByText("sk-live-secret")).not.toBeInTheDocument();
    expect(screen.getByText(/value: \[hidden\]/)).toBeInTheDocument();
  });
});
