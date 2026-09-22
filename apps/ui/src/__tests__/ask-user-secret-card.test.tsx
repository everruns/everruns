import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AskUserToolCall, type AskUserArguments } from "@/components/chat/ask-user-tool-call";

const submitQuestionAnswers = jest.fn();
const setSessionSecret = jest.fn();

jest.mock("@/lib/api/sessions", () => ({
  submitQuestionAnswers: (...args: unknown[]) => submitQuestionAnswers(...args),
  setSessionSecret: (...args: unknown[]) => setSessionSecret(...args),
}));

const CREDENTIAL = "rk_live_thisisthesecretvalue";

const request: AskUserArguments = {
  questions: [
    {
      id: "stripe_key",
      kind: "secret",
      header: "Stripe key",
      question: "Which Stripe restricted key should I use?",
      options: [],
      secret_name: "STRIPE_API_KEY",
      purpose: "Read-only charge lookups for the reconciliation report.",
    },
  ],
  asked_at: "2026-09-20T05:00:00Z",
};

function renderCard(overrides: Partial<AskUserArguments> = {}) {
  return render(
    <AskUserToolCall
      sessionId="session_1"
      toolCallId="ask_1"
      request={{ ...request, ...overrides }}
      requestedAt="2026-09-20T05:00:00Z"
      toolResultsMap={new Map()}
    />,
  );
}

beforeEach(() => {
  submitQuestionAnswers.mockReset().mockResolvedValue({});
  setSessionSecret.mockReset().mockResolvedValue(undefined);
});

describe("ask_user secret question", () => {
  it("collects the credential in a password field, not a text box", () => {
    const { container } = renderCard();
    expect(screen.getByText("The agent needs a credential")).toBeInTheDocument();
    expect(
      screen.getByText("Read-only charge lookups for the reconciliation report."),
    ).toBeInTheDocument();
    expect(container.querySelector('input[type="password"]')).not.toBeNull();
    expect(container.querySelector('input[type="radio"]')).toBeNull();
    expect(container.querySelector('input[type="checkbox"]')).toBeNull();
  });

  /// THREAT[TM-AGENT-016]: the value goes to the encrypted store; the question
  /// is answered with a reference. Nothing that reaches the event log has it.
  it("sends the value only to the secret store and answers with a reference", async () => {
    const { container } = renderCard();
    const field = container.querySelector('input[type="password"]') as HTMLInputElement;
    fireEvent.change(field, { target: { value: CREDENTIAL } });
    fireEvent.click(screen.getByRole("button", { name: "Store and continue" }));

    await waitFor(() => expect(submitQuestionAnswers).toHaveBeenCalled());

    expect(setSessionSecret).toHaveBeenCalledWith("session_1", "STRIPE_API_KEY", CREDENTIAL);
    const [, answerPayload] = submitQuestionAnswers.mock.calls[0];
    expect(answerPayload).toEqual({
      tool_call_id: "ask_1",
      status: "answered",
      answers: [{ id: "stripe_key", secret_ref: "session:STRIPE_API_KEY" }],
    });
    expect(JSON.stringify(answerPayload)).not.toContain(CREDENTIAL);
  });

  it("cannot submit an empty credential", () => {
    renderCard();
    expect(screen.getByRole("button", { name: "Store and continue" })).toBeDisabled();
  });

  it("declines without storing anything", async () => {
    renderCard();
    fireEvent.click(screen.getByRole("button", { name: "Decline" }));

    await waitFor(() => expect(submitQuestionAnswers).toHaveBeenCalled());
    expect(setSessionSecret).not.toHaveBeenCalled();
    expect(submitQuestionAnswers.mock.calls[0][1]).toEqual({
      tool_call_id: "ask_1",
      status: "declined",
      answers: [],
    });
  });

  it("says the question does not time out", () => {
    renderCard();
    expect(screen.getByText(/does not time out/)).toBeInTheDocument();
    expect(screen.queryByText(/Continuing with/)).toBeNull();
  });

  /// A call that claims `secret` but omits what the card needs must not reach
  /// the password path.
  it("does not render the password path for a malformed secret question", () => {
    const { container } = renderCard({
      questions: [
        {
          id: "stripe_key",
          kind: "secret",
          header: "Stripe key",
          question: "Which key?",
          options: [],
        },
      ],
    });
    expect(container.querySelector('input[type="password"]')).toBeNull();
    expect(screen.queryByText("The agent needs a credential")).toBeNull();
  });
});
