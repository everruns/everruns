jest.mock("@/providers/feature-flags-provider", () => ({ useFeatureFlag: () => true }));
import { render, screen } from "@testing-library/react";
import type { Event } from "@/lib/api/types";
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: jest.fn() }),
}));

jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({ t: (key: string) => key }),
}));

const mockSessionContext = {
  agent: { name: "platform-chat" },
  sessionId: "session_123",
  session: {
    id: "session_123",
    title: "Recorded run",
    tags: ["recording"],
  },
  events: [] as Event[],
  eventsLoading: false,
};

jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  useSessionContext: () => mockSessionContext,
}));

jest.mock("@/components/events/event-filter", () => ({
  EventFilter: () => <div>event filters</div>,
}));

import EventsPage from "@/app/(main)/sessions/[sessionId]/events/page";

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

  it("keeps Events as the exact raw ledger rather than either projection", () => {
    mockSessionContext.events = [rawEvent("custom.debug", 42)];
    render(<EventsPage />);

    expect(screen.getByText("custom.debug")).toBeInTheDocument();
    expect(screen.getByText(/raw_worker_id/)).toBeInTheDocument();
    expect(screen.getByText(/worker_abc/)).toBeInTheDocument();
    expect(screen.getByText(/exec_abc/)).toBeInTheDocument();
    expect(screen.getByText(/event_42/)).toBeInTheDocument();
  });
});
