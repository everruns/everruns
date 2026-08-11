import { render, screen } from "@testing-library/react";
import type { Event } from "@/lib/api/types";

const mockSessionContext = {
  sessionId: "session_123",
  events: [] as Event[],
  eventsLoading: false,
};

jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  useSessionContext: () => mockSessionContext,
}));

jest.mock("@/hooks/use-session-tasks", () => ({
  useSessionTasks: () => ({ data: [], isLoading: false }),
}));

jest.mock("@/components/session/session-transcript", () => ({
  SessionTranscript: () => <div data-testid="session-transcript">conversation</div>,
}));

jest.mock("@/components/session/run-timeline", () => ({
  RunTimeline: ({ events }: { events: Event[] }) => (
    <div data-testid="run-timeline">{events.map((event) => event.type).join(",")}</div>
  ),
}));

jest.mock("@/components/events/event-filter", () => ({
  EventFilter: () => <div>event filters</div>,
}));

import EventsPage from "@/app/(main)/sessions/[sessionId]/events/page";
import TimelinePage from "@/app/(main)/sessions/[sessionId]/timeline/page";
import TranscriptPage from "@/app/(main)/sessions/[sessionId]/transcript/page";

function rawEvent(type: string, sequence: number): Event {
  return {
    id: `event_${sequence}`,
    session_id: "session_123",
    type,
    sequence,
    ts: "2026-08-10T12:00:00Z",
    context: { exec_id: "exec_abc" },
    data: { raw_worker_id: "worker_abc", nested: { complete: true } },
  } as unknown as Event;
}

describe("session recording projections", () => {
  beforeEach(() => {
    mockSessionContext.events = [];
    mockSessionContext.eventsLoading = false;
  });

  it("renders Transcript as the full conversation projection", () => {
    render(<TranscriptPage />);

    expect(screen.getByTestId("session-transcript")).toBeInTheDocument();
    expect(screen.queryByTestId("run-timeline")).not.toBeInTheDocument();
  });

  it("renders Timeline full-width without a transcript rail", () => {
    mockSessionContext.events = [rawEvent("turn.started", 1)];
    const { container } = render(<TimelinePage />);

    expect(screen.getByRole("region", { name: "Execution timeline" })).toHaveClass("flex-1");
    expect(screen.getByTestId("run-timeline")).toHaveTextContent("turn.started");
    expect(screen.queryByTestId("session-transcript")).not.toBeInTheDocument();
    expect(container.querySelector("aside")).not.toBeInTheDocument();
  });

  it("updates Timeline when live events arrive and replays completed history", () => {
    const { rerender } = render(<TimelinePage />);
    expect(screen.getByTestId("run-timeline")).toBeEmptyDOMElement();

    mockSessionContext.events = [
      rawEvent("turn.started", 1),
      rawEvent("llm.generation", 2),
      rawEvent("turn.completed", 3),
    ];
    rerender(<TimelinePage />);

    expect(screen.getByTestId("run-timeline")).toHaveTextContent(
      "turn.started,llm.generation,turn.completed",
    );
  });

  it("keeps Events as the exact raw ledger rather than either projection", () => {
    mockSessionContext.events = [rawEvent("custom.debug", 42)];
    render(<EventsPage />);

    expect(screen.getByText("custom.debug")).toBeInTheDocument();
    expect(screen.getByText(/raw_worker_id/)).toBeInTheDocument();
    expect(screen.getByText(/worker_abc/)).toBeInTheDocument();
    expect(screen.getByText(/exec_abc/)).toBeInTheDocument();
    expect(screen.getByText(/event_42/)).toBeInTheDocument();
    expect(screen.queryByTestId("session-transcript")).not.toBeInTheDocument();
    expect(screen.queryByTestId("run-timeline")).not.toBeInTheDocument();
  });
});
