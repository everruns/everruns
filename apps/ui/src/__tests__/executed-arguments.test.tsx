import { fireEvent, render, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import { BashToolCallCard } from "@/components/chat/bash-tool-call-card";
import { getExecutedArguments } from "@/components/chat/executed-arguments";
import { ToolActivityRow } from "@/components/chat/tool-activity-row";
import { ToolActivityTimelineGroup } from "@/components/chat/tool-activity-timeline-group";
import type { ToolCompletedData } from "@/lib/api/types";
import { LocaleProvider } from "@/providers/locale-provider";

function renderWithLocale(ui: ReactElement) {
  return render(<LocaleProvider>{ui}</LocaleProvider>);
}

function completed(overrides: Partial<ToolCompletedData> = {}): ToolCompletedData {
  return {
    tool_call_id: "call-1",
    tool_name: "http_request",
    success: true,
    status: "success",
    result: [{ type: "text", text: "ok" }],
    ...overrides,
  };
}

const ORIGINAL = { url: "https://example.com/a", token: "[REDACTED]" };
const EXECUTED = { url: "https://proxy.internal/a", token: "[REDACTED]" };

describe("getExecutedArguments", () => {
  it("returns null when no hook rewrote the arguments", () => {
    expect(getExecutedArguments(completed())).toBeNull();
    expect(getExecutedArguments(undefined)).toBeNull();
  });

  it("pretty-prints object values and keeps truncated strings verbatim", () => {
    expect(getExecutedArguments(completed({ executed_arguments: { a: 1 } }))).toEqual({
      text: '{\n  "a": 1\n}',
      truncated: false,
    });
    expect(
      getExecutedArguments(
        completed({ executed_arguments: '{"a":"xxxx', executed_arguments_truncated: true }),
      ),
    ).toEqual({ text: '{"a":"xxxx', truncated: true });
  });
});

describe("executed arguments in the session timeline", () => {
  it("shows the hook indicator and executed arguments beside the original", () => {
    renderWithLocale(
      <ToolActivityTimelineGroup
        headline="Fetched page"
        rows={[
          {
            id: "call-1",
            label: "Fetch page",
            state: "completed",
            arguments: ORIGINAL,
            result: completed({ executed_arguments: EXECUTED }),
          },
        ]}
      />,
    );

    const toggle = screen.getByRole("button", { name: /arguments rewritten by hook/i });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Executed arguments")).not.toBeInTheDocument();

    fireEvent.click(toggle);

    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Original arguments")).toBeInTheDocument();
    expect(screen.getByText("Executed arguments")).toBeInTheDocument();
    expect(screen.getByText(/proxy\.internal/)).toBeInTheDocument();
    expect(screen.getByText(/example\.com\/a/)).toBeInTheDocument();
    expect(screen.queryByText(/truncated preview/i)).not.toBeInTheDocument();
  });

  it("renders nothing extra when the arguments were not rewritten", () => {
    renderWithLocale(
      <ToolActivityTimelineGroup
        headline="Fetched page"
        rows={[
          {
            id: "call-1",
            label: "Fetch page",
            state: "completed",
            arguments: ORIGINAL,
            result: completed(),
          },
        ]}
      />,
    );

    expect(screen.queryByText(/arguments rewritten by hook/i)).not.toBeInTheDocument();
    expect(screen.queryByTestId("executed-arguments")).not.toBeInTheDocument();
  });

  it("shows a truncated note for a truncated string preview", () => {
    renderWithLocale(
      <ToolActivityTimelineGroup
        headline="Fetched page"
        rows={[
          {
            id: "call-1",
            label: "Fetch page",
            state: "completed",
            result: completed({
              executed_arguments: '{"body":"aaaaaaaa',
              executed_arguments_truncated: true,
            }),
          },
        ]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /arguments rewritten by hook/i }));

    expect(screen.getByText('{"body":"aaaaaaaa')).toBeInTheDocument();
    expect(screen.getByText(/truncated preview/i)).toBeInTheDocument();
    // No original arguments known for this row: only the executed section renders.
    expect(screen.queryByText("Original arguments")).not.toBeInTheDocument();
  });
});

describe("executed arguments in tool cards", () => {
  it("marks a rewritten generic tool activity row", () => {
    renderWithLocale(
      <ToolActivityRow
        toolCall={{ id: "call-1", name: "http_request", arguments: ORIGINAL }}
        toolResult={completed({ executed_arguments: EXECUTED })}
        mode="server"
        locale="en"
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /arguments rewritten by hook/i }));
    expect(screen.getByText(/proxy\.internal/)).toBeInTheDocument();
  });

  it("marks a rewritten bash command and leaves an unrewritten one alone", () => {
    const toolCall = { id: "call-1", name: "bash", arguments: { command: "rm -rf build" } };
    const { unmount } = renderWithLocale(
      <BashToolCallCard
        toolCall={toolCall}
        toolResult={completed({
          tool_name: "bash",
          executed_arguments: { command: "rm -rf ./build --one-file-system" },
        })}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /arguments rewritten by hook/i }));
    expect(screen.getByText(/one-file-system/)).toBeInTheDocument();
    unmount();

    renderWithLocale(
      <BashToolCallCard toolCall={toolCall} toolResult={completed({ tool_name: "bash" })} />,
    );
    expect(screen.queryByText(/arguments rewritten by hook/i)).not.toBeInTheDocument();
  });
});
