import { render, screen } from "@testing-library/react";
import {
  DesktopRecordingCards,
  assignRecordingsToEvents,
  sessionFileDownloadUrl,
} from "@/components/chat/desktop-recordings";
import type { Event } from "@/lib/api/types";

let sequence = 0;

function event(type: string, ts: string, data: Record<string, unknown>, turnId?: string): Event {
  sequence += 1;
  return {
    id: `evt_${sequence}`,
    type,
    ts,
    session_id: "sess_1",
    context: turnId ? { turn_id: turnId } : {},
    data,
  } as Event;
}

function turn(turnId: string, startTs: string, endTs: string) {
  const input = event(
    "input.message",
    startTs,
    { message: { id: `msg_${turnId}`, content: [{ type: "text", text: "hi" }] } },
    turnId,
  );
  const started = event(
    "turn.started",
    startTs,
    { turn_id: turnId, input_message_id: `msg_${turnId}` },
    turnId,
  );
  const answer = event(
    "output.message.completed",
    endTs,
    { message: { id: `out_${turnId}`, content: [{ type: "text", text: "done" }] } },
    turnId,
  );
  const completed = event(
    "turn.completed",
    endTs,
    { turn_id: turnId, iterations: 1, duration_ms: 1000 },
    turnId,
  );
  return { events: [input, started, answer, completed], answerId: answer.id };
}

function written(path: string, ts: string, size = 2048): Event {
  return event("file.written", ts, { path, operation: "create", size_bytes: size, created: true });
}

describe("assignRecordingsToEvents", () => {
  it("places a recording under the turn that ended before it was saved", () => {
    const first = turn("turn_1", "2026-10-26T10:00:00Z", "2026-10-26T10:01:00Z");
    const second = turn("turn_2", "2026-10-26T10:05:00Z", "2026-10-26T10:06:00Z");
    const events = [
      ...first.events,
      written("/recordings/desktop-a.mp4", "2026-10-26T10:01:05Z"),
      ...second.events,
      written("/recordings/desktop-b.mp4", "2026-10-26T10:06:03Z", 4096),
    ];

    const placed = assignRecordingsToEvents(events);

    expect(placed.get(first.answerId)).toEqual([
      { path: "/recordings/desktop-a.mp4", sizeBytes: 2048 },
    ]);
    expect(placed.get(second.answerId)).toEqual([
      { path: "/recordings/desktop-b.mp4", sizeBytes: 4096 },
    ]);
  });

  it("ignores other files and recordings saved before any turn ended", () => {
    const only = turn("turn_1", "2026-10-26T10:00:00Z", "2026-10-26T10:01:00Z");
    const events = [
      written("/recordings/early.mp4", "2026-10-26T09:59:00Z"),
      ...only.events,
      written("/reports/summary.md", "2026-10-26T10:01:05Z"),
      written("/recordings/notes.txt", "2026-10-26T10:01:06Z"),
    ];

    expect(assignRecordingsToEvents(events).size).toBe(0);
  });
});

describe("DesktopRecordingCards", () => {
  it("plays the recording from the session's raw file download", () => {
    const { container } = render(
      <DesktopRecordingCards
        sessionId="session_1"
        recordings={[{ path: "/recordings/desktop a.mp4", sizeBytes: 3 * 1024 * 1024 }]}
      />,
    );

    const url = sessionFileDownloadUrl("session_1", "/recordings/desktop a.mp4");
    expect(url).toMatch(
      /\/v1\/sessions\/session_1\/fs\/_\/download\/recordings\/desktop%20a\.mp4$/,
    );
    expect(container.querySelector("video")?.getAttribute("src")).toBe(url);
    expect(screen.getByText("Desktop recording")).toBeInTheDocument();
    expect(screen.getByText(/3\.0 MB/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Download/ })).toHaveAttribute("href", url);
  });
});
