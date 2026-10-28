import { fireEvent, render, screen } from "@testing-library/react";
import { StepRow } from "@/components/session/trace/trace-rows";
import type { TraceStep, TraceTurn } from "@/lib/api/types";

jest.mock("@/components/chat/streamdown-message", () => ({
  StreamdownMessage: ({ children }: { children: string }) => (
    <div data-testid="markdown">
      <a href="https://example.com">{children}</a>
    </div>
  ),
}));

const step: TraceStep & { type: "step" } = {
  type: "step",
  turn: 1,
  step: 1,
  kind: "answer",
  status: "success",
  started_at: "2026-10-10T00:00:00Z",
  offset_ms: 0,
  duration_ms: 50,
  start_sequence: 1,
  narration: "# Answer\n\n**Hello**",
};
const turn: TraceTurn = {
  turn: 1,
  turn_id: "turn_1",
  status: "completed",
  started_at: step.started_at,
  duration_ms: 1000,
  step_count: 1,
  model_calls: 1,
  tool_calls: 0,
  subagent_calls: 0,
  error_count: 0,
  input_tokens: 0,
  output_tokens: 0,
  start_sequence: 1,
  items: [step],
};

it("switches between Markdown and the original text without selecting the row when a link is used", () => {
  const onSelect = jest.fn();
  const { rerender } = render(
    <StepRow
      turn={turn}
      step={step}
      view="messages"
      plainText={false}
      selected={false}
      onSelect={onSelect}
    />,
  );
  expect(screen.getByTestId("markdown")).toHaveTextContent("# Answer");
  fireEvent.click(screen.getByRole("link"));
  expect(onSelect).not.toHaveBeenCalled();
  rerender(
    <StepRow
      turn={turn}
      step={step}
      view="messages"
      plainText
      selected={false}
      onSelect={onSelect}
    />,
  );
  expect(screen.queryByTestId("markdown")).not.toBeInTheDocument();
  expect(screen.getByText("# Answer **Hello**")).toHaveClass("whitespace-pre-wrap");
  expect(screen.getByText("# Answer **Hello**").textContent).toBe(step.narration);
  fireEvent.click(screen.getByRole("button"));
  expect(onSelect).toHaveBeenCalledTimes(1);
});
