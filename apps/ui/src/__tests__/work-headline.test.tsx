import { render, screen } from "@testing-library/react";
import { WorkHeadline } from "@/components/chat/work-headline";
import type { WorkReport } from "@/lib/api/types";

const mockUseLiveWorkReport = jest.fn();
jest.mock("@/hooks/use-turn-work-report", () => ({
  useLiveWorkReport: (...args: unknown[]) => mockUseLiveWorkReport(...args),
}));
jest.mock("@/providers/locale-provider", () => {
  const { formatMessage } = jest.requireActual("@/lib/i18n");
  return {
    useLocale: () => ({
      locale: "en",
      t: (key: string, values?: Record<string, string | number>) =>
        formatMessage("en", key, values),
    }),
  };
});

const running: WorkReport = {
  turn_id: "turn_1",
  state: "working",
  headline: "Running `cargo test` 7 of 12",
  steps: [
    {
      id: "c1",
      tool_name: "bash",
      label: "Running `cargo test`",
      state: "running",
      started_at: "2026-10-23T00:00:00Z",
    },
  ],
} as WorkReport;

beforeEach(() => mockUseLiveWorkReport.mockReset());

describe("WorkHeadline", () => {
  it("says what a live session is doing, with the plan step when it shared one", () => {
    mockUseLiveWorkReport.mockReturnValue({
      ...running,
      plan: { items: [], done: 2, total: 4, current: "Run the tests" },
    });
    render(<WorkHeadline sessionId="session_1" live fallback="Running" />);

    expect(mockUseLiveWorkReport).toHaveBeenCalledWith("session_1", true);
    expect(screen.getByTestId("work-headline")).toHaveTextContent(
      "Step 3 of 4 · Running `cargo test` 7 of 12",
    );
  });

  it("names what a parked session waits on", () => {
    mockUseLiveWorkReport.mockReturnValue({
      ...running,
      state: "needs_you",
      steps: [],
      attention: { kind: "approval", prompt: "Deploy to staging", tool_call_ids: ["c2"] },
    });
    render(<WorkHeadline sessionId="session_1" live fallback="Running" />);

    expect(screen.getByTestId("work-headline")).toHaveTextContent(
      "Needs your approval · Deploy to staging",
    );
  });

  it("keeps the fallback until there is something to say", () => {
    mockUseLiveWorkReport.mockReturnValue(null);
    render(<WorkHeadline sessionId="session_1" live={false} fallback="Succeeded" />);

    expect(mockUseLiveWorkReport).toHaveBeenCalledWith("session_1", false);
    expect(screen.getByText("Succeeded")).toBeInTheDocument();
    expect(screen.queryByTestId("work-headline")).not.toBeInTheDocument();
  });
});
