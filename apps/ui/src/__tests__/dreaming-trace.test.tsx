import { render, screen } from "@testing-library/react";
import { useQuery } from "@tanstack/react-query";
import { DreamingTrace } from "@/components/session/trace/dreaming-trace";

jest.mock("@tanstack/react-query", () => ({ useQuery: jest.fn() }));
const query = useQuery as jest.Mock;

test("does not add an empty background lane", () => {
  query.mockReturnValue({ data: [], error: null });
  const { container } = render(<DreamingTrace sessionId="session_test" />);
  expect(container).toBeEmptyDOMElement();
});

test("groups statuses by operation without rendering memory content", () => {
  query.mockReturnValue({
    data: [
      {
        ts: "2026-10-19T00:00:00Z",
        data: { operation_id: "operation_1", status: "started", note_count: 0 },
      },
      {
        ts: "2026-10-19T00:00:02Z",
        data: {
          operation_id: "operation_1",
          status: "generated",
          note_count: 0,
          usage: {
            input_tokens: 100,
            output_tokens: 30,
            cache_read_tokens: 25,
            cache_creation_tokens: 10,
            actual_cost_usd: 0.009,
          },
          notes: ["private memory text"],
          evidence: "private evidence",
        },
      },
      {
        ts: "2026-10-19T00:00:04Z",
        data: {
          operation_id: "operation_1",
          status: "completed",
          note_count: 2,
          cost_usd: 0.01,
          model: "test-model",
        },
      },
    ],
    error: null,
  });
  render(<DreamingTrace sessionId="session_test" />);
  expect(screen.getByText(/1 recent operation/)).toBeInTheDocument();
  expect(screen.getByText("completed")).toBeInTheDocument();
  expect(screen.getByText("2 notes")).toBeInTheDocument();
  expect(screen.getByText("Latest call reserved maximum: $0.0100")).toBeInTheDocument();
  expect(
    screen.getByText(/100 uncached input.*25 cache read.*10 cache write.*30 output/),
  ).toBeInTheDocument();
  expect(screen.getByText(/Reported cost: \$0.0090/)).toBeInTheDocument();
  expect(screen.getByText("Elapsed: 4.0s")).toBeInTheDocument();
  expect(screen.queryByText(/private memory text|private evidence/)).not.toBeInTheDocument();
});
