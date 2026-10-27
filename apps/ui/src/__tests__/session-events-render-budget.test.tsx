import { render, cleanup, fireEvent, screen, waitFor } from "@testing-library/react";
import EventsPage from "@/app/(main)/sessions/[sessionId]/events/page";
import type { Event } from "@/lib/api/types";
import { formatEventPreview, RawEventPreview } from "@/components/events/raw-event-preview";

const mockSessionState = { events: [] as Event[], eventsLoading: false, sessionId: "fixture" };
jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  useSessionContext: () => mockSessionState,
}));
jest.mock("@/components/chat-mode/page-view-state", () => ({
  ...jest.requireActual("@/components/chat-mode/page-view-state"),
  useChatPageFilters: () => {},
}));
jest.mock("@/components/llm/llm-history-viewer", () => ({ LlmHistoryViewer: () => null }));

afterEach(cleanup);

test("bounds the visible raw payload for screenshot-heavy session events", () => {
  // Synthetic data only. A fixed 3.2 MiB visible page, not a production replay.
  const screenshot = "A".repeat(64 * 1024);
  mockSessionState.events = Array.from({ length: 445 }, (_, sequence) => ({
    id: `fixture-${sequence}`,
    session_id: "fixture",
    context: {},
    sequence,
    type: "tool.completed",
    ts: "2026-10-26T00:00:00Z",
    data: sequence >= 395 ? { screenshot, label: "desktop fixture" } : { status: "ok" },
  })) as Event[];
  const started = performance.now();
  const cpu = process.cpuUsage();
  const memory = process.memoryUsage().heapUsed;
  const { container } = render(<EventsPage />);
  const payloads = Array.from(container.querySelectorAll("pre"));
  console.info(
    "events-render-fixture",
    JSON.stringify({
      events: 445,
      visibleRows: payloads.length,
      renderedCharacters: payloads.reduce((sum, node) => sum + (node.textContent?.length ?? 0), 0),
      renderMs: performance.now() - started,
      cpuMicros: process.cpuUsage(cpu),
      heapDeltaBytes: process.memoryUsage().heapUsed - memory,
    }),
  );
  expect(payloads).toHaveLength(50);
  for (const payload of payloads) expect(payload.textContent!.length).toBeLessThanOrEqual(8192);
});

test("keeps complete, ordinary JSON readable without a truncation notice", () => {
  const event = { id: "small", data: { result: "done", enabled: true, count: 2 } };
  expect(formatEventPreview(event)).toEqual({
    text: JSON.stringify(event, null, 2),
    truncated: false,
  });
});

test.each([
  { nested: Array.from({ length: 1000 }, () => ({ value: "short" })) },
  Object.fromEntries(Array.from({ length: 1000 }, (_, i) => [`key-${i}`, "value"])),
  { escaped: "\u0000".repeat(100_000) },
  { nested: { a: { b: { c: { d: { e: { f: { g: "deep" } } } } } } } },
])("bounds traversal, depth and escaped output", (event) => {
  const preview = formatEventPreview(event);
  expect(preview.truncated).toBe(true);
  expect(preview.text.length).toBeLessThanOrEqual(8192);
});

test("does not traverse the rest of a wide payload after its preview budget", () => {
  const unread = jest.fn(() => "must not inspect");
  const data = Object.fromEntries(Array.from({ length: 1000 }, (_, i) => [`field-${i}`, i]));
  Object.defineProperty(data, "outside-budget", { enumerable: true, get: unread });
  expect(formatEventPreview(data).truncated).toBe(true);
  expect(unread).not.toHaveBeenCalled();
});

test("reuses immutable previews across unrelated parent updates", () => {
  const read = jest.fn(() => "small");
  const event = { data: Object.defineProperty({}, "value", { enumerable: true, get: read }) };
  const { rerender } = render(<RawEventPreview event={event} />);
  expect(read).toHaveBeenCalledTimes(1);
  rerender(<RawEventPreview event={event} />);
  expect(read).toHaveBeenCalledTimes(1);
});

test("copy retains the complete original event even when its preview is shortened", async () => {
  const writeText = jest.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  mockSessionState.events = [
    {
      id: "large",
      session_id: "fixture",
      context: {},
      type: "tool.completed",
      ts: "2026-10-26T00:00:00Z",
      data: { screenshot: "A".repeat(64 * 1024) },
    },
  ] as Event[];
  render(<EventsPage />);
  expect(screen.getByText("Preview shortened. Copy includes the complete event.")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: /^Copy$/ }));
  await waitFor(() =>
    expect(writeText).toHaveBeenCalledWith(JSON.stringify(mockSessionState.events[0], null, 2)),
  );
});

test("renders untrusted markup as text", () => {
  const { container } = render(
    <RawEventPreview event={{ data: "<img src=x onerror=alert(1)>" }} />,
  );
  expect(container.querySelector("img")).toBeNull();
  expect(container.textContent).toContain("<img src=x onerror=alert(1)>");
});
