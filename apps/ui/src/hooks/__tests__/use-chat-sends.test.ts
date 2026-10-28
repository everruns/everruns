import { act, renderHook, waitFor } from "@testing-library/react";
import type { Event } from "@/lib/api/types";
import type { ChatSendInput, CreatedMessage } from "@/lib/api/messages";
import { useChatSends } from "../use-chat-sends";
import { CLIENT_MESSAGE_ID_METADATA_KEY, NOT_DELIVERED_MS } from "@/lib/chat-turn-state";
import { chatRecoveryKey, readPendingSends, clearChatRecovery } from "@/lib/chat-recovery";

let seq = 0;

function stored(id: string, clientId: string): Event {
  seq += 1;
  return {
    id: `ev_${seq}`,
    type: "input.message",
    ts: new Date().toISOString(),
    session_id: "ses_1",
    sequence: seq,
    context: {},
    data: {
      message: {
        id,
        role: "user",
        content: [{ type: "text", text: "hi" }],
        metadata: { [CLIENT_MESSAGE_ID_METADATA_KEY]: clientId },
      },
    },
  } as unknown as Event;
}

function turnStarted(turnId: string, messageId: string): Event {
  seq += 1;
  return {
    id: `ev_${seq}`,
    type: "turn.started",
    ts: new Date().toISOString(),
    session_id: "ses_1",
    sequence: seq,
    context: { turn_id: turnId },
    data: { turn_id: turnId, input_message_id: messageId },
  } as unknown as Event;
}

function message(id: string, delivery: CreatedMessage["delivery"] = "started"): CreatedMessage {
  return {
    id,
    session_id: "ses_1",
    sequence: 1,
    role: "user",
    content: [],
    tool_call_id: null,
    created_at: new Date().toISOString(),
    delivery,
  };
}

function setup(send: (input: ChatSendInput, signal: AbortSignal) => Promise<CreatedMessage>) {
  const cancel = jest.fn(async () => undefined);
  const hook = renderHook(
    ({ events }: { events: Event[] }) => useChatSends({ sessionId: "ses_1", events, send, cancel }),
    { initialProps: { events: [] as Event[] } },
  );
  return { ...hook, cancel };
}

beforeEach(() => {
  seq = 0;
  sessionStorage.clear();
});

it("recovers an interrupted send after unmount with its original identity and attachments", async () => {
  const storageKey = chatRecoveryKey("alice", "org", "ses_1");
  const send = jest.fn<Promise<CreatedMessage>, [ChatSendInput, AbortSignal]>(async () => {
    throw new TypeError("network");
  });
  const options = { sessionId: "ses_1", storageKey, events: [], send, cancel: jest.fn() };
  const first = renderHook(() => useChatSends(options));
  await act(async () => {
    await first.result.current
      .submit({ text: "keep me", files: [{ fileId: "file_1" }] })
      .catch(() => {});
  });
  const clientId = first.result.current.pending[0].clientId;
  first.unmount();
  const second = renderHook(() => useChatSends(options));
  expect(second.result.current.pending[0]).toMatchObject({
    clientId,
    text: "keep me",
    phase: "failed",
    files: [{ fileId: "file_1" }],
  });
  act(() => window.dispatchEvent(new Event("online")));
  expect(send).toHaveBeenCalledTimes(1); // Restored intent needs review.
  act(() => second.result.current.retry(clientId));
  await waitFor(() => expect(send).toHaveBeenCalledTimes(2));
  expect(send.mock.calls[1][0]).toMatchObject({ clientMessageId: clientId });
});

it("never exposes the previous account's pending sends when the recovery scope changes", async () => {
  const send = jest.fn(async () => {
    throw new Error("rejected");
  });
  const { result, rerender } = renderHook(
    ({ storageKey }) =>
      useChatSends({ sessionId: "ses_1", storageKey, events: [], send, cancel: jest.fn() }),
    { initialProps: { storageKey: chatRecoveryKey("alice", "org", "ses_1") } },
  );
  await act(async () => {
    await result.current.submit({ text: "private" }).catch(() => {});
  });
  rerender({ storageKey: chatRecoveryKey("bob", "org", "ses_1") });
  expect(result.current.pending).toEqual([]);
});

it("retries connectivity failures on network return under the original id, once", async () => {
  const send = jest
    .fn<Promise<CreatedMessage>, [ChatSendInput, AbortSignal]>()
    .mockRejectedValueOnce(new TypeError("Failed to fetch"))
    .mockResolvedValueOnce(message("msg_1"));
  const { result } = setup(send);
  await act(async () => {
    await result.current.submit({ text: "hi" }).catch(() => {});
  });
  const clientId = result.current.pending[0].clientId;
  await act(async () => {
    window.dispatchEvent(new Event("online"));
    window.dispatchEvent(new Event("online"));
  });
  expect(send).toHaveBeenCalledTimes(2);
  expect(send.mock.calls[1][0].clientMessageId).toBe(clientId);
});

it("does not replay a server rejection when the network returns", async () => {
  const send = jest.fn(async () => {
    throw new Error("permission denied");
  });
  const { result } = setup(send);
  await act(async () => {
    await result.current.submit({ text: "hi" }).catch(() => {});
  });
  await act(async () => {
    window.dispatchEvent(new Event("online"));
  });
  expect(send).toHaveBeenCalledTimes(1);
});

it("removes acknowledged sends from recovery before the stream echoes them", async () => {
  const storageKey = chatRecoveryKey("alice", "org", "ses_1");
  const { result } = renderHook(() =>
    useChatSends({
      sessionId: "ses_1",
      storageKey,
      events: [],
      send: async () => message("msg_1"),
      cancel: jest.fn(),
    }),
  );
  await act(async () => {
    await result.current.submit({ text: "hi" });
  });
  expect(readPendingSends(storageKey)).toEqual([]);
});

it("tracks a send from Enter to its stored event", async () => {
  const send = jest.fn(async (_input: ChatSendInput, _signal: AbortSignal) =>
    message("msg_1", "steered"),
  );
  const { result, rerender } = setup(send);

  await act(async () => {
    await result.current.submit({ text: "hi" });
  });

  const clientId = result.current.pending[0].clientId;
  expect(send.mock.calls[0][0]).toMatchObject({ text: "hi", clientMessageId: clientId });
  expect(result.current.pending[0]).toMatchObject({
    phase: "acked",
    messageId: "msg_1",
    delivery: "steered",
  });
  expect(result.current.busy).toBe(true);

  rerender({ events: [stored("msg_1", clientId)] });
  expect(result.current.pending).toEqual([]);
});

it("reports a failed send and retries it under the same id", async () => {
  const send = jest
    .fn<Promise<CreatedMessage>, [ChatSendInput, AbortSignal]>()
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValueOnce(message("msg_1"));
  const { result } = setup(send);

  await act(async () => {
    await result.current.submit({ text: "hi" }).catch(() => undefined);
  });
  expect(result.current.pending[0]).toMatchObject({ phase: "failed", error: "offline" });
  expect(result.current.busy).toBe(false);

  const clientId = result.current.pending[0].clientId;
  act(() => result.current.retry(clientId));
  await waitFor(() => expect(result.current.pending[0].phase).toBe("acked"));
  expect(send.mock.calls[1][0].clientMessageId).toBe(clientId);
});

it("retries with the page context captured at the original send", async () => {
  const send = jest
    .fn<Promise<CreatedMessage>, [ChatSendInput, AbortSignal]>()
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValueOnce(message("msg_1"));
  const { result } = setup(send);
  const metadata = {
    page_context: { route: "/agents/agent_1", entity_id: "agent_1" },
    page_actions: [{ label: "Saved edits" }],
  };
  await act(async () => {
    await result.current.submit({ text: "Improve this", metadata }).catch(() => undefined);
  });
  const clientId = result.current.pending[0].clientId;
  act(() => result.current.retry(clientId));
  await waitFor(() => expect(result.current.pending[0].phase).toBe("acked"));
  expect(send.mock.calls[1][0]).toMatchObject({ clientMessageId: clientId, metadata });
});

it("gives the draft back when stopped before the server acks", async () => {
  const send = jest.fn(
    (_input: ChatSendInput, signal: AbortSignal) =>
      new Promise<CreatedMessage>((_resolve, reject) => {
        signal.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
      }),
  );
  const { result } = setup(send);

  let sending: Promise<CreatedMessage> = Promise.resolve(message("unused"));
  act(() => {
    sending = result.current.submit({ text: "never mind" });
  });
  let stopped: ReturnType<typeof result.current.stop> = false;
  await act(async () => {
    stopped = result.current.stop();
    await sending.catch(() => undefined);
  });

  expect(stopped).toMatchObject({ text: "never mind" });
  expect(result.current.pending).toEqual([]);
});

it("holds a stop after the ack until the turn starts, then cancels it", async () => {
  const send = jest.fn(async () => message("msg_1"));
  const { result, rerender, cancel } = setup(send);

  await act(async () => {
    await result.current.submit({ text: "hi" });
  });
  const clientId = result.current.pending[0].clientId;
  let stopped: ReturnType<typeof result.current.stop> = false;
  act(() => {
    stopped = result.current.stop();
  });
  expect(stopped).toBe(true);
  expect(cancel).not.toHaveBeenCalled();

  const input = stored("msg_1", clientId);
  rerender({ events: [input] });
  expect(cancel).not.toHaveBeenCalled();
  rerender({ events: [input, turnStarted("turn_1", "msg_1")] });
  expect(cancel).toHaveBeenCalledTimes(1);
});

it("reports a send with no ack in time as not delivered", async () => {
  jest.useFakeTimers();
  try {
    const send = jest.fn(
      (_input: ChatSendInput, signal: AbortSignal) =>
        new Promise<CreatedMessage>((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(new DOMException("timeout", "AbortError")));
        }),
    );
    const { result } = setup(send);
    let sending: Promise<CreatedMessage> = Promise.resolve(message("unused"));
    act(() => {
      sending = result.current.submit({ text: "hi" });
    });
    await act(async () => {
      jest.advanceTimersByTime(NOT_DELIVERED_MS);
      await sending.catch(() => undefined);
    });
    expect(result.current.pending[0].phase).toBe("failed");
  } finally {
    jest.useRealTimers();
  }
});

it.each(["client echo before ack", "stored id after ack"])(
  "never renders a pending send beside its stored message: %s",
  async (order) => {
    let acknowledge!: (value: CreatedMessage) => void;
    const send = () =>
      new Promise<CreatedMessage>((resolve) => {
        acknowledge = resolve;
      });
    const cancel = jest.fn(async () => undefined);
    const renderedMessageCounts: number[] = [];
    const { result, rerender } = renderHook(
      ({ events }: { events: Event[] }) => {
        const sends = useChatSends({ sessionId: "ses_1", events, send, cancel });
        renderedMessageCounts.push(events.length + sends.pending.length);
        return sends;
      },
      { initialProps: { events: [] as Event[] } },
    );
    let sending!: Promise<CreatedMessage>;
    act(() => {
      sending = result.current.submit({ text: "hi" });
    });
    const clientId = result.current.pending[0].clientId;
    if (order === "stored id after ack") {
      await act(async () => {
        acknowledge(message("msg_1"));
        await sending;
      });
    }
    renderedMessageCounts.length = 0;
    const input = stored("msg_1", order === "client echo before ack" ? clientId : "legacy-client");
    rerender({ events: [input] });
    if (order === "client echo before ack") {
      await act(async () => {
        acknowledge(message("msg_1"));
        await sending;
      });
    }
    // Observe every render, including the frame before effect-based cleanup.
    expect(renderedMessageCounts.length).toBeGreaterThan(0);
    expect(renderedMessageCounts.every((count) => count === 1)).toBe(true);
    expect(result.current.busy).toBe(false);
  },
);

it("a late network failure cannot repopulate recovery after logout", async () => {
  const storageKey = chatRecoveryKey("alice", "org", "ses_1");
  let reject!: (reason: Error) => void;
  const send = () =>
    new Promise<CreatedMessage>((_resolve, failure) => {
      reject = failure;
    });
  const { result } = renderHook(() =>
    useChatSends({ sessionId: "ses_1", storageKey, events: [], send, cancel: jest.fn() }),
  );
  let request!: Promise<CreatedMessage>;
  act(() => {
    request = result.current.submit({ text: "private" });
  });
  clearChatRecovery();
  await act(async () => {
    reject(new TypeError("network"));
    await request.catch(() => {});
  });
  expect(sessionStorage.getItem(storageKey)).toBeNull();
});
