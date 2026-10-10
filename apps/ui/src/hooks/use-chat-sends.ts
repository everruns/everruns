/**
 * Pending chat sends: the client half of the turn status row
 * (knowledge/ui/chat-experience.md).
 *
 * Decisions:
 * - A send exists from Enter. It carries a client-minted id the server stores
 *   with the message, so a retry is safe (the server answers a repeat with the
 *   message it already has) and the stored event reconciles by id, not text.
 * - No ack within NOT_DELIVERED_MS aborts the request and reports the send as
 *   not delivered; Retry resends under the same id.
 * - Stop before the ack abandons the request and hands the text back to the
 *   composer. If the server stored it anyway, its turn is cancelled as soon as
 *   it starts. Stop after the ack but before the turn starts waits for that
 *   start too: cancelling earlier would race the turn it means to stop.
 */
"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ChatSendInput } from "@/lib/api/messages";
import type { Event, Message } from "@/lib/api/types";
import { getEventData } from "@/lib/api/types";
import {
  getClientMessageId,
  newClientMessageId,
  NOT_DELIVERED_MS,
  reconcilePendingSends,
  type PendingSend,
} from "@/lib/chat-turn-state";

export type ChatSendDraft = Omit<ChatSendInput, "clientMessageId">;

export interface ChatSends {
  /** Sends the event stream does not carry yet, oldest first. */
  pending: PendingSend[];
  /** Whether a send is in flight or waiting for its turn to start. */
  busy: boolean;
  /** Send now; resolves with the stored message, rejects when not delivered. */
  submit: (draft: ChatSendDraft) => Promise<Message>;
  retry: (clientId: string) => void;
  discard: (clientId: string) => void;
  /**
   * Stop the newest send that has no running turn yet. Returns the draft to
   * put back in the composer when the send never reached the server, `true`
   * when a stop is held until the turn starts, or `false` when there was
   * nothing to stop (the caller cancels the running turn instead).
   */
  stop: () => ChatSendDraft | boolean;
}

interface UseChatSendsOptions {
  sessionId: string;
  events: Event[] | undefined;
  send: (input: ChatSendInput, signal: AbortSignal) => Promise<Message>;
  cancel: () => Promise<unknown>;
}

function draftOf(send: PendingSend): ChatSendDraft {
  return {
    text: send.text,
    images: send.images,
    files: send.files,
    controls: send.controls,
    addressedParticipantId: send.addressedParticipantId,
  };
}

export function useChatSends({ sessionId, events, send, cancel }: UseChatSendsOptions): ChatSends {
  const [pending, setPending] = useState<PendingSend[]>([]);
  const pendingRef = useRef(pending);
  useEffect(() => {
    pendingRef.current = pending;
  }, [pending]);
  const controllers = useRef(new Map<string, AbortController>());
  // Client ids the user stopped before an ack, and message ids stopped before
  // their turn started: cancel the turn that answers either.
  const stoppedClientIds = useRef(new Set<string>());
  const stoppedMessageIds = useRef(new Set<string>());

  const update = useCallback((clientId: string, patch: Partial<PendingSend>) => {
    setPending((current) =>
      current.map((item) => (item.clientId === clientId ? { ...item, ...patch } : item)),
    );
  }, []);

  const attempt = useCallback(
    async (item: PendingSend): Promise<Message> => {
      const controller = new AbortController();
      controllers.current.set(item.clientId, controller);
      let timedOut = false;
      const timer = window.setTimeout(() => {
        timedOut = true;
        controller.abort();
      }, NOT_DELIVERED_MS);
      try {
        const message = await send(
          { clientMessageId: item.clientId, ...draftOf(item) },
          controller.signal,
        );
        update(item.clientId, {
          phase: "acked",
          ackAtMs: Date.now(),
          messageId: message.id,
          delivery: message.delivery,
        });
        return message;
      } catch (error) {
        if (controller.signal.aborted && !timedOut) {
          // Stopped by the user: the composer already has the text back.
          setPending((current) => current.filter((p) => p.clientId !== item.clientId));
        } else {
          update(item.clientId, {
            phase: "failed",
            error: error instanceof Error ? error.message : String(error),
          });
        }
        throw error;
      } finally {
        window.clearTimeout(timer);
        controllers.current.delete(item.clientId);
      }
    },
    [send, update],
  );

  const submit = useCallback(
    (draft: ChatSendDraft) => {
      const now = Date.now();
      const item: PendingSend = {
        clientId: newClientMessageId(now),
        text: draft.text,
        images: draft.images ?? [],
        files: draft.files ?? [],
        controls: draft.controls,
        addressedParticipantId: draft.addressedParticipantId,
        phase: "sending",
        sentAtMs: now,
      };
      setPending((current) => [...current, item]);
      return attempt(item);
    },
    [attempt],
  );

  const retry = useCallback(
    (clientId: string) => {
      const item = pendingRef.current.find((p) => p.clientId === clientId);
      if (!item || item.phase !== "failed") return;
      const next: PendingSend = {
        ...item,
        phase: "sending",
        sentAtMs: Date.now(),
        error: undefined,
      };
      update(clientId, next);
      attempt(next).catch(() => undefined);
    },
    [attempt, update],
  );

  const discard = useCallback((clientId: string) => {
    setPending((current) => current.filter((p) => p.clientId !== clientId));
  }, []);

  const stop = useCallback((): ChatSendDraft | boolean => {
    for (let i = pendingRef.current.length - 1; i >= 0; i -= 1) {
      const item = pendingRef.current[i];
      if (item.phase === "sending") {
        stoppedClientIds.current.add(item.clientId);
        controllers.current.get(item.clientId)?.abort();
        return draftOf(item);
      }
      if (item.phase === "acked" && item.messageId && item.delivery !== "steered") {
        stoppedMessageIds.current.add(item.messageId);
        return true;
      }
    }
    return false;
  }, []);

  // Drop sends the stream now carries, and cancel turns the user already stopped.
  useEffect(() => {
    if (!events?.length) return;
    setPending((current) => reconcilePendingSends(current, events));

    if (stoppedClientIds.current.size === 0 && stoppedMessageIds.current.size === 0) return;
    for (const event of events) {
      if (event.type !== "input.message") continue;
      const clientId = getClientMessageId(event);
      const messageId = getEventData(event, "input.message")?.message?.id;
      if (clientId && messageId && stoppedClientIds.current.delete(clientId)) {
        stoppedMessageIds.current.add(messageId);
      }
    }
    for (const event of events) {
      const started = getEventData(event, "turn.started");
      if (!started || !stoppedMessageIds.current.delete(started.input_message_id)) continue;
      const turnEnded = events.some(
        (e) =>
          e.context?.turn_id === started.turn_id &&
          (e.type === "turn.completed" || e.type === "turn.failed" || e.type === "turn.cancelled"),
      );
      if (!turnEnded) cancel().catch(() => undefined);
    }
  }, [cancel, events]);

  // A session's sends never follow it to another session.
  useEffect(() => {
    const live = controllers.current;
    const stoppedSends = stoppedClientIds.current;
    const stoppedTurns = stoppedMessageIds.current;
    return () => {
      for (const controller of live.values()) controller.abort();
      live.clear();
      stoppedSends.clear();
      stoppedTurns.clear();
      setPending([]);
    };
  }, [sessionId]);

  const busy = pending.some((p) => p.phase === "sending" || p.phase === "acked");

  return useMemo(
    () => ({ pending, busy, submit, retry, discard, stop }),
    [pending, busy, submit, retry, discard, stop],
  );
}
