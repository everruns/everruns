import { fireEvent, render, screen } from "@testing-library/react";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import type { Event, InputMessageData } from "@/lib/api/types";
import { getTextFromContent } from "@/lib/api/types";
import { CLIENT_MESSAGE_ID_METADATA_KEY, type PendingSend } from "@/lib/chat-turn-state";

jest.mock("streamdown", () => ({
  Streamdown: ({ children }: { children: string }) => <div>{children}</div>,
}));
jest.mock("@streamdown/code", () => ({ code: {} }));
jest.mock("@streamdown/mermaid", () => ({ createMermaidPlugin: jest.fn(() => ({})) }));
jest.mock("remark-gfm", () => jest.fn());
jest.mock("remark-github-blockquote-alert", () => jest.fn());
jest.mock("@/hooks", () => ({
  useAgents: () => ({ data: [] }),
  useProviders: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-members", () => ({ useMembers: () => ({ data: [] }) }));
jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({ locale: "en", t: (key: string) => key }),
}));
jest.mock("@/components/chat/message-info-icon", () => ({ MessageInfoIcon: () => null }));

let seq = 0;
const now = Date.now();

function ev(type: string, data: unknown, turnId?: string): Event {
  seq += 1;
  return {
    id: `ev_${seq}`,
    type,
    ts: new Date(now + seq).toISOString(),
    session_id: "ses_1",
    sequence: seq,
    context: turnId ? { turn_id: turnId } : {},
    data,
  } as unknown as Event;
}

function input(id: string, text: string, clientId?: string): Event {
  return ev("input.message", {
    message: {
      id,
      role: "user",
      content: [{ type: "text", text }],
      metadata: clientId ? { [CLIENT_MESSAGE_ID_METADATA_KEY]: clientId } : {},
    },
  });
}

const pending = (overrides: Partial<PendingSend> = {}): PendingSend => ({
  clientId: "client-1",
  text: "Ship it",
  images: [],
  files: [],
  phase: "sending",
  sentAtMs: Date.now(),
  ...overrides,
});

function List({
  events,
  pendingSends = [],
  sessionActive = true,
  onRetrySend,
}: {
  events: Event[];
  pendingSends?: PendingSend[];
  sessionActive?: boolean;
  onRetrySend?: (clientId: string) => void;
}) {
  return (
    <ChatMessageList
      events={events}
      chatEvents={events.filter((e) => e.type === "input.message")}
      sessionId="ses_1"
      toolResultsMap={new Map()}
      toolProgressMap={new Map()}
      toolOutputMap={new Map()}
      eventsLoading={false}
      hasMoreEvents={false}
      loadingOlderEvents={false}
      getMessageText={(data) => getTextFromContent((data as InputMessageData).message.content)}
      getToolCalls={() => []}
      pendingSends={pendingSends}
      onRetrySend={onRetrySend}
      sessionActive={sessionActive}
    />
  );
}

const rowOf = (container: HTMLElement) =>
  container.querySelector("[data-turn-phase]") as HTMLElement | null;

beforeEach(() => {
  seq = 0;
});

it("shows Sending under the message from Enter", () => {
  const { container } = render(<List events={[]} pendingSends={[pending()]} />);
  expect(screen.getByText("Ship it")).toBeInTheDocument();
  expect(rowOf(container)?.dataset.turnPhase).toBe("sending");
  expect(screen.getByText("turn_sending")).toBeInTheDocument();
});

it("keeps the same row element when the send is stored and its turn runs", () => {
  const { container, rerender } = render(<List events={[]} pendingSends={[pending()]} />);
  const row = rowOf(container);

  rerender(
    <List
      events={[]}
      pendingSends={[pending({ phase: "acked", ackAtMs: Date.now(), messageId: "msg_1" })]}
    />,
  );
  expect(rowOf(container)).toBe(row);
  expect(row?.dataset.turnPhase).toBe("starting");

  const stored = input("msg_1", "Ship it", "client-1");
  rerender(<List events={[stored]} />);
  expect(rowOf(container)).toBe(row);

  const started = ev("turn.started", { turn_id: "turn_1", input_message_id: "msg_1" }, "turn_1");
  const reason = ev("reason.started", {}, "turn_1");
  rerender(<List events={[stored, started, reason]} />);
  expect(rowOf(container)).toBe(row);
  expect(row?.dataset.turnPhase).toBe("thinking");

  const done = ev(
    "turn.completed",
    { turn_id: "turn_1", iterations: 1, duration_ms: 3000 },
    "turn_1",
  );
  rerender(<List events={[stored, started, reason, done]} sessionActive={false} />);
  expect(rowOf(container)).toBe(row);
  expect(row?.dataset.turnPhase).toBe("done");
});

it("shows a message sent mid-turn as queued", () => {
  const first = input("msg_1", "Start");
  const started = ev("turn.started", { turn_id: "turn_1", input_message_id: "msg_1" }, "turn_1");
  const reason = ev("reason.started", {}, "turn_1");
  const steered = input("msg_2", "Also this");
  const { container } = render(<List events={[first, started, reason, steered]} />);

  const phases = Array.from(container.querySelectorAll("[data-turn-phase]")).map(
    (node) => (node as HTMLElement).dataset.turnPhase,
  );
  expect(phases).toEqual(["thinking", "queued"]);
  expect(screen.getByText("turn_queued_status")).toBeInTheDocument();
});

it("offers Retry on a send that was not delivered", () => {
  const onRetrySend = jest.fn();
  render(
    <List events={[]} pendingSends={[pending({ phase: "failed" })]} onRetrySend={onRetrySend} />,
  );
  expect(screen.getByText("turn_not_delivered")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /turn_retry_send/ }));
  expect(onRetrySend).toHaveBeenCalledWith("client-1");
});
