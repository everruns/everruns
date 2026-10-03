import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AskUserToolCall, type AskUserArguments } from "@/components/chat/ask-user-tool-call";
import { buildToolActivityGroups } from "@/components/chat/tool-activity-groups";
import type { Event, ToolCompletedData } from "@/lib/api/types";

const submitQuestionAnswers = jest.fn();

jest.mock("@/lib/api/sessions", () => ({
  submitQuestionAnswers: (...args: unknown[]) => submitQuestionAnswers(...args),
}));

const request: AskUserArguments = {
  questions: [
    {
      id: "target",
      header: "Target",
      question: "Which environment should I deploy to?",
      allow_other: true,
      options: [
        {
          label: "Staging",
          description: "Safe and reversible.",
          default: true,
        },
        {
          label: "Production",
          description: "Serves live traffic.",
        },
      ],
    },
  ],
  asked_at: "2026-09-20T05:00:00Z",
  nudge_at: "2026-09-20T05:04:00Z",
  expires_at: "2026-09-20T05:05:00Z",
};

function renderCard(
  overrides: Partial<AskUserArguments> = {},
  toolResultsMap = new Map<string, ToolCompletedData>(),
) {
  return render(
    <AskUserToolCall
      sessionId="session_1"
      toolCallId="ask_1"
      request={{ ...request, ...overrides }}
      requestedAt="2026-09-20T05:00:00Z"
      toolResultsMap={toolResultsMap}
    />,
  );
}

function completedResult(result: Record<string, unknown>): ToolCompletedData {
  return {
    tool_call_id: "ask_1",
    tool_name: "ask_user",
    success: true,
    status: "success",
    result: [{ type: "text", text: JSON.stringify(result) }],
  };
}

beforeEach(() => {
  jest.spyOn(Date, "now").mockReturnValue(Date.parse("2026-09-20T05:04:13Z"));
  submitQuestionAnswers.mockReset();
  submitQuestionAnswers.mockResolvedValue({
    answered_by: "user",
    session_status: "active",
    status: "answered",
  });
});

afterEach(() => {
  jest.restoreAllMocks();
});

describe("AskUserToolCall", () => {
  it.each([
    ["MacIntel", "⌘"],
    ["Win32", "Ctrl"],
  ])("shows numbered keycaps with the %s shortcut", (platform, modifier) => {
    jest.spyOn(navigator, "platform", "get").mockReturnValue(platform);
    const { container } = renderCard();
    expect(container.querySelectorAll("kbd")).toHaveLength(0);
    fireEvent.keyDown(document, {
      key: platform === "MacIntel" ? "Meta" : "Control",
      [platform === "MacIntel" ? "metaKey" : "ctrlKey"]: true,
    });
    expect(Array.from(container.querySelectorAll("kbd"), (key) => key.textContent)).toEqual([
      `${modifier}+1`,
      `${modifier}+2`,
      `${modifier}+3`,
    ]);
    expect(
      screen.getByText(`Hold ${modifier} to show option shortcuts (1–9).`),
    ).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Other" })).toHaveAttribute(
      "aria-keyshortcuts",
      "Meta+3 Control+3",
    );
  });

  it("numbers long MCP choices without advertising shortcuts beyond nine", () => {
    const { container } = renderCard({
      mcp_elicitation: { server: "deploys", message: "Pick a target." },
      questions: [
        {
          ...request.questions[0],
          options: Array.from({ length: 10 }, (_, index) => ({
            label: `Target ${index + 1}`,
            description: "Deployment target",
          })),
        },
      ],
    });
    fireEvent.keyDown(document, { key: "Control", ctrlKey: true });
    expect(container.querySelectorAll("kbd")).toHaveLength(9);
    expect(screen.getByRole("radio", { name: /^Target 9 / })).toHaveAttribute(
      "aria-keyshortcuts",
      "Meta+9 Control+9",
    );
    expect(screen.getByRole("radio", { name: /^Target 10 / })).not.toHaveAttribute(
      "aria-keyshortcuts",
    );
    expect(screen.getByRole("radio", { name: "Other" })).not.toHaveAttribute("aria-keyshortcuts");
    fireEvent.keyDown(document, { code: "Digit9", ctrlKey: true });
    expect(screen.getByRole("radio", { name: /^Target 9 / })).toBeChecked();
  });

  it("hides shortcut badges on modifier release, window blur, and visibility changes", () => {
    const { container } = renderCard();
    for (const release of [
      () => fireEvent.keyUp(document, { key: "Meta", metaKey: false }),
      () => fireEvent.blur(window),
      () => fireEvent(document, new window.Event("visibilitychange")),
    ]) {
      fireEvent.keyDown(document, { key: "Meta", metaKey: true });
      expect(container.querySelectorAll("kbd")).toHaveLength(3);
      release();
      expect(container.querySelectorAll("kbd")).toHaveLength(0);
    }
  });

  it.each(["metaKey", "ctrlKey"])("selects by %s+number without submitting", (modifier) => {
    renderCard();
    fireEvent.keyDown(document, { code: "Digit2", key: "2", [modifier]: true });
    expect(screen.getByRole("radio", { name: /Production/ })).toBeChecked();
    expect(screen.getByRole("radio", { name: /Staging/ })).not.toBeChecked();
    expect(screen.getByRole("radio", { name: /Production/ })).toHaveFocus();
    expect(submitQuestionAnswers).not.toHaveBeenCalled();
  });

  it("leaves typing, other modifiers, repeats, composition, and unrelated keys alone", () => {
    renderCard();
    for (const keys of [
      { code: "Digit2", key: "2" },
      { code: "Digit2", metaKey: true, shiftKey: true },
      { code: "Digit2", ctrlKey: true, shiftKey: true },
      { code: "Digit2", metaKey: true, altKey: true },
      { code: "Digit2", metaKey: true, repeat: true },
      { code: "Digit2", metaKey: true, isComposing: true },
      { code: "Digit9", metaKey: true },
    ]) {
      expect(fireEvent.keyDown(document, keys)).toBe(true);
    }
    expect(screen.getByRole("radio", { name: /Staging/ })).toBeChecked();
    expect(submitQuestionAnswers).not.toHaveBeenCalled();
  });

  it("toggles multi-select choices in the focused question without changing another question", () => {
    renderCard({
      questions: [
        request.questions[0],
        {
          id: "checks",
          header: "Checks",
          question: "Which checks?",
          multi_select: true,
          options: [
            { label: "Unit", description: "Unit tests" },
            { label: "UI", description: "UI tests" },
          ],
        },
      ],
    });
    const unit = screen.getByRole("checkbox", { name: /Unit/ });
    fireEvent.keyDown(unit, { code: "Digit2", ctrlKey: true });
    const ui = screen.getByRole("checkbox", { name: /UI/ });
    expect(ui).toBeChecked();
    expect(screen.getByRole("radio", { name: /Staging/ })).toBeChecked();
    fireEvent.keyDown(ui, { code: "Digit2", ctrlKey: true });
    expect(ui).not.toBeChecked();
  });

  it("numbers Other and opens its text field by keyboard", () => {
    renderCard();
    fireEvent.keyDown(document, { code: "Digit3", metaKey: true });
    expect(screen.getByRole("radio", { name: "Other" })).toBeChecked();
    expect(screen.getByRole("textbox", { name: "Target other answer" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    expect(submitQuestionAnswers).not.toHaveBeenCalled();
  });

  it("does not steal shortcuts from a dialog or password field", () => {
    renderCard();
    render(
      <>
        <div role="dialog">
          <input aria-label="Dialog field" />
        </div>
        <input type="password" aria-label="Credential" />
      </>,
    );
    for (const name of ["Dialog field", "Credential"]) {
      expect(
        fireEvent.keyDown(screen.getByLabelText(name), {
          code: "Digit2",
          metaKey: true,
        }),
      ).toBe(true);
    }
    expect(screen.getByRole("radio", { name: /Staging/ })).toBeChecked();
  });

  it("only changes the focused pending card when multiple cards are mounted", () => {
    const first = renderCard();
    const second = renderCard();
    const secondChoice = within(second.container).getByRole("radio", { name: /Staging/ });
    fireEvent.keyDown(secondChoice, { code: "Digit2", metaKey: true });
    expect(within(second.container).getByRole("radio", { name: /Production/ })).toBeChecked();
    expect(within(first.container).getByRole("radio", { name: /Staging/ })).toBeChecked();
  });

  it("does not intercept shortcuts while submitting or after completion", async () => {
    submitQuestionAnswers.mockReturnValue(new Promise(() => {}));
    const { rerender } = renderCard();
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(fireEvent.keyDown(document, { code: "Digit2", metaKey: true })).toBe(true);
    expect(screen.getByRole("radio", { name: /Staging/ })).toBeChecked();
    rerender(
      <AskUserToolCall
        sessionId="session_1"
        toolCallId="ask_1"
        request={request}
        requestedAt={request.asked_at!}
        toolResultsMap={new Map([["ask_1", completedResult({ status: "declined" })]])}
      />,
    );
    expect(screen.getByText("Questions declined")).toBeInTheDocument();
    expect(fireEvent.keyDown(document, { code: "Digit2", metaKey: true })).toBe(true);
  });

  it("shows option descriptions, selection mode, recommendation, and the named countdown", () => {
    renderCard();

    expect(screen.getByText("Which environment should I deploy to?")).toBeInTheDocument();
    expect(
      screen.getByRole("group", { name: "Which environment should I deploy to?" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Safe and reversible.")).toBeInTheDocument();
    expect(screen.getByText("Serves live traffic.")).toBeInTheDocument();
    expect(screen.getByText("Recommended")).toBeInTheDocument();
    expect(screen.getAllByRole("radio")).toHaveLength(3);
    expect(screen.getByText("Continuing with Staging in 0:47")).toBeInTheDocument();
  });

  it("attributes an MCP server's questions to the server and declines on timeout", () => {
    renderCard({ mcp_elicitation: { server: "deploys", message: "Pick a target." } });

    expect(screen.getByText("Questions from MCP server deploys")).toBeInTheDocument();
    expect(screen.getByText(/Everruns is not asking these/)).toBeInTheDocument();
    expect(screen.getByText("Pick a target.")).toBeInTheDocument();
    // The server's default is neither endorsed nor preselected.
    expect(screen.queryByText("Recommended")).not.toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /Staging/ })).not.toBeChecked();
    expect(screen.getByText("Declining deploys's questions in 0:47")).toBeInTheDocument();
    expect(screen.queryByText(/Continuing with/)).not.toBeInTheDocument();
  });

  it("keeps the timeout choice hidden before the nudge deadline", () => {
    jest.mocked(Date.now).mockReturnValue(Date.parse("2026-09-20T05:03:59Z"));
    renderCard();

    expect(screen.queryByText(/Continuing with/)).not.toBeInTheDocument();
  });

  it("renders and submits a text question without choice affordances", async () => {
    renderCard({
      questions: [
        {
          kind: "text",
          id: "branch_name",
          header: "Branch",
          question: "What should I call this branch?",
          options: [],
        },
      ],
    });

    const input = screen.getByRole("textbox", { name: "Branch answer" });
    expect(input.tagName).toBe("TEXTAREA");
    expect(input).toHaveAttribute("placeholder", "Type your answer");
    expect(screen.queryByRole("radio")).not.toBeInTheDocument();
    expect(screen.queryByText("Recommended")).not.toBeInTheDocument();
    expect(screen.getByText("Skipping unanswered questions in 0:47")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();

    fireEvent.change(input, { target: { value: "feature/open-question" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    await waitFor(() =>
      expect(submitQuestionAnswers).toHaveBeenCalledWith("session_1", {
        tool_call_id: "ask_1",
        status: "answered",
        answers: [
          {
            id: "branch_name",
            selected: [],
            other_text: "feature/open-question",
          },
        ],
      }),
    );
    expect(screen.getByText("Answered: feature/open-question")).toBeInTheDocument();
  });

  it("reveals Other and submits its free text through the typed answer endpoint", async () => {
    renderCard();

    fireEvent.click(screen.getByRole("radio", { name: "Other" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Target other answer" }), {
      target: { value: "A canary environment" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    await waitFor(() =>
      expect(submitQuestionAnswers).toHaveBeenCalledWith("session_1", {
        tool_call_id: "ask_1",
        status: "answered",
        answers: [
          {
            id: "target",
            selected: [],
            other_text: "A canary environment",
          },
        ],
      }),
    );
    expect(screen.getByText("Answered: A canary environment")).toBeInTheDocument();
  });

  it("records a decline without sending answers", async () => {
    renderCard();

    fireEvent.click(screen.getByRole("button", { name: "Decline" }));

    await waitFor(() =>
      expect(submitQuestionAnswers).toHaveBeenCalledWith("session_1", {
        tool_call_id: "ask_1",
        status: "declined",
        answers: [],
      }),
    );
    expect(screen.getByText("Questions declined")).toBeInTheDocument();
  });

  it("submits one payload for stacked single-select and multi-select questions", async () => {
    renderCard({
      questions: [
        request.questions[0],
        {
          id: "checks",
          header: "Checks",
          question: "Which checks should run?",
          multi_select: true,
          allow_other: false,
          options: [
            { label: "Unit", description: "Run focused unit tests." },
            { label: "UI", description: "Run browser verification." },
          ],
        },
      ],
    });

    fireEvent.click(screen.getByRole("checkbox", { name: /Unit/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /UI/ }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    await waitFor(() =>
      expect(submitQuestionAnswers).toHaveBeenCalledWith("session_1", {
        tool_call_id: "ask_1",
        status: "answered",
        answers: [
          { id: "target", selected: ["Staging"], other_text: null },
          { id: "checks", selected: ["Unit", "UI"], other_text: null },
        ],
      }),
    );
  });

  it.each([
    ["answered", "user", "Answered: Staging"],
    ["declined", "user", "Questions declined"],
    ["timed_out", "timeout", "Auto-selected: Staging"],
    ["cancelled", "unattended", "Questions cancelled"],
  ] as const)("renders %s tool results as a terminal summary", (status, answeredBy, summary) => {
    const answers =
      status === "answered" || status === "timed_out"
        ? [{ id: "target", selected: ["Staging"], other_text: null }]
        : [];
    renderCard(
      {},
      new Map([
        [
          "ask_1",
          completedResult({
            status,
            answered_by: answeredBy,
            answers,
          }),
        ],
      ]),
    );

    expect(screen.getByText(summary)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Continue" })).not.toBeInTheDocument();
  });

  it("leaves ask_user to this card rather than the activity timeline", () => {
    const event: Event = {
      id: "request",
      type: "tool.call_requested",
      ts: "2026-09-20T05:00:00Z",
      session_id: "session_1",
      context: { turn_id: "turn-1", exec_id: "exec-1" },
      data: {
        tool_calls: [{ id: "ask_1", name: "ask_user", arguments: request }],
        tool_summaries: [{ id: "ask_1", name: "ask_user" }],
      },
    };

    const groups = buildToolActivityGroups([event], "Working");
    expect(groups.byAnchorEventId.size).toBe(0);
    expect(groups.groupedEventIds.has("request")).toBe(false);
  });
});
