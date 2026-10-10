import { act, renderHook, waitFor } from "@testing-library/react";
import type { Event } from "@/lib/api/types";
import type { ChatSendInput, CreatedMessage } from "@/lib/api/messages";
import { useChatSends } from "../use-chat-sends";
import { CLIENT_MESSAGE_ID_METADATA_KEY, NOT_DELIVERED_MS } from "@/lib/chat-turn-state";

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
