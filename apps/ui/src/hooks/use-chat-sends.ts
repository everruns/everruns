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
import type { ChatSendInput, CreatedMessage } from "@/lib/api/messages";
import type { Event } from "@/lib/api/types";
import { chatRecoveryEpoch, readPendingSends, writeChatRecovery } from "@/lib/chat-recovery";
import { getEventData } from "@/lib/api/types";
import {
  getClientMessageId,
  isChatRetryExpired,
  newClientMessageId,
  NOT_DELIVERED_MS,
  reconcilePendingSends,
  type PendingSend,
} from "@/lib/chat-turn-state";

const EMPTY_PENDING: PendingSend[] = [];

export type ChatSendDraft = Omit<ChatSendInput, "clientMessageId">;

export interface ChatSends {
  storageKey?: string;
  /** Storage could not be read or updated; keep the tab open. */
  recoveryError?: boolean;
  /** Sends the event stream does not carry yet, oldest first. */
  pending: PendingSend[];
  /** Whether a send is in flight or waiting for its turn to start. */
  busy: boolean;
  /** Send now; resolves with the stored message, rejects when not delivered. */
  submit: (draft: ChatSendDraft) => Promise<CreatedMessage>;
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
  storageKey?: string;
  events: Event[] | undefined;
  send: (input: ChatSendInput, signal: AbortSignal) => Promise<CreatedMessage>;
  cancel: () => Promise<unknown>;
}

function draftOf(send: PendingSend): ChatSendDraft {
  return {
    metadata: send.metadata,
    text: send.text,
    images: send.images,
    files: send.files,
    controls: send.controls,
    addressedParticipantId: send.addressedParticipantId,
  };
}

export function useChatSends({
  sessionId,
  storageKey,
  events,
  send,
  cancel,
}: UseChatSendsOptions): ChatSends {
  const [state, setState] = useState<{ key?: string; session: string; items: PendingSend[] }>({
    key: storageKey,
    session: sessionId,
    items: [],
  });
  const [recoveryError, setRecoveryError] = useState(false);
  const pendingRef = useRef<PendingSend[]>([]);
  const generation = useRef(0);
  const recoveryEpoch = useRef(chatRecoveryEpoch());
  const recovering = useRef(false);
  const pending =
    state.key === storageKey && state.session === sessionId ? state.items : EMPTY_PENDING;
  // Persist synchronously, before starting the request. Never perform storage
  // writes inside React state updaters (which StrictMode may invoke twice).
  const commit = useCallback(
    (next: PendingSend[] | ((items: PendingSend[]) => PendingSend[])) => {
      const items = typeof next === "function" ? next(pendingRef.current) : next;
      if (items === pendingRef.current) return;
      pendingRef.current = items;
      try {
        writeChatRecovery(storageKey, { pending: items }, recoveryEpoch.current);
        setRecoveryError(false);
      } catch {
        setRecoveryError(true);
      }
      setState({ key: storageKey, session: sessionId, items });
    },
    [storageKey, sessionId],
  );
  const controllers = useRef(new Map<string, AbortController>());
  // Client ids the user stopped before an ack, and message ids stopped before
  // their turn started: cancel the turn that answers either.
  const stoppedClientIds = useRef(new Set<string>());
  const stoppedMessageIds = useRef(new Set<string>());

  const update = useCallback(
    (clientId: string, patch: Partial<PendingSend>) => {
      commit((current) =>
        current.map((item) => (item.clientId === clientId ? { ...item, ...patch } : item)),
      );
    },
    [commit],
  );

  const attempt = useCallback(
    async (item: PendingSend): Promise<CreatedMessage> => {
      const epoch = generation.current;
      const controller = new AbortController();
      controllers.current.set(item.clientId, controller);
      let timedOut = false;
      const timer = window.setTimeout(() => {
        timedOut = true;
        controller.abort();
      }, NOT_DELIVERED_MS);
      try {
        if (recoveryEpoch.current !== chatRecoveryEpoch())
          throw new Error("recovery_scope_retired");
        if (!navigator.onLine) throw new TypeError("offline");
        const message = await send(
          { clientMessageId: item.clientId, ...draftOf(item) },
          controller.signal,
        );
        if (epoch !== generation.current) return message;
        update(item.clientId, {
          phase: "acked",
          ackAtMs: Date.now(),
          messageId: message.id,
          delivery: message.delivery,
        });
        return message;
      } catch (error) {
        if (epoch !== generation.current) throw error;
        if (controller.signal.aborted && !timedOut) {
          // Stopped by the user: the composer already has the text back.
          commit((current) => current.filter((p) => p.clientId !== item.clientId));
        } else {
          update(item.clientId, {
            phase: "failed",
            retryWhenOnline: error instanceof TypeError,
            error: error instanceof Error ? error.message : String(error),
          });
        }
        throw error;
      } finally {
        window.clearTimeout(timer);
        if (controllers.current.get(item.clientId) === controller)
          controllers.current.delete(item.clientId);
      }
    },
    [send, update, commit],
  );

  const submit = useCallback(
    (draft: ChatSendDraft) => {
      const now = Date.now();
      const item: PendingSend = {
        metadata: draft.metadata,
        clientId: newClientMessageId(now),
        text: draft.text,
        images: draft.images ?? [],
        files: draft.files ?? [],
        controls: draft.controls,
        addressedParticipantId: draft.addressedParticipantId,
        phase: "sending",
        sentAtMs: now,
        createdAtMs: now,
      };
      commit((current) => [...current, item]);
      return attempt(item);
    },
    [attempt, commit],
  );

  const retry = useCallback(
    (clientId: string) => {
      const item = pendingRef.current.find((p) => p.clientId === clientId);
      if (!item || item.phase !== "failed" || controllers.current.has(clientId)) return;
      if (isChatRetryExpired(item)) {
        update(clientId, { retryExpired: true, retryWhenOnline: false });
        return;
      }
      const next: PendingSend = {
        ...item,
        phase: "sending",
        createdAtMs: item.createdAtMs ?? item.sentAtMs,
        sentAtMs: Date.now(),
        error: undefined,
        retryWhenOnline: false,
      };
      update(clientId, next);
      attempt(next).catch(() => undefined);
    },
    [attempt, update],
  );

  const discard = useCallback(
    (clientId: string) => {
      commit((current) => current.filter((p) => p.clientId !== clientId));
    },
    [commit],
  );

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

  useEffect(() => {
    generation.current += 1;
    recoveryEpoch.current = chatRecoveryEpoch();
    let restored: PendingSend[] = [];
    try {
      restored = readPendingSends(storageKey);
      setRecoveryError(false);
    } catch {
      setRecoveryError(true);
    }
    pendingRef.current = restored;
    setState((current) =>
      current.key === storageKey &&
      current.session === sessionId &&
      current.items.length === 0 &&
      restored.length === 0
        ? current
        : { key: storageKey, session: sessionId, items: restored },
    );
    const live = controllers.current;
    const stoppedSends = stoppedClientIds.current;
    const stoppedTurns = stoppedMessageIds.current;
    return () => {
      // Unmount abort is not a user discard. Keep the already-saved request
      // identity, and prevent late acknowledgments from touching another scope.
      generation.current += 1;
      for (const controller of live.values()) controller.abort();
      live.clear();
      stoppedSends.clear();
      stoppedTurns.clear();
    };
  }, [sessionId, storageKey]);

  // Drop sends the stream now carries, and cancel turns the user already stopped.
  useEffect(() => {
    if (!events?.length) return;
    commit((current) => reconcilePendingSends(current, events));

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
  }, [cancel, events, commit]);

  useEffect(() => {
    const recover = async () => {
      if (!navigator.onLine || recovering.current || controllers.current.size) return;
      recovering.current = true;
      const epoch = generation.current;
      try {
        // Serial replay preserves authored order and never overlaps a manual retry.
        for (const queued of [...pendingRef.current]) {
          if (epoch !== generation.current) break;
          const item = pendingRef.current.find((entry) => entry.clientId === queued.clientId);
          if (
            !item ||
            !item.retryWhenOnline ||
            item.phase !== "failed" ||
            controllers.current.has(item.clientId)
          )
            continue;
          if (isChatRetryExpired(item)) {
            update(item.clientId, { retryExpired: true, retryWhenOnline: false });
            continue;
          }
          const next = {
            ...item,
            phase: "sending" as const,
            createdAtMs: item.createdAtMs ?? item.sentAtMs,
            retryWhenOnline: false,
            sentAtMs: Date.now(),
            error: undefined,
          };
          update(item.clientId, next);
          try {
            await attempt(next);
          } catch {
            break;
          }
        }
      } finally {
        recovering.current = false;
      }
    };
    const wake = () => {
      void recover();
    };
    window.addEventListener("online", wake);
    return () => window.removeEventListener("online", wake);
  }, [attempt, update]);

  const visiblePending = useMemo(
    () => reconcilePendingSends(pending, events ?? []),
    [pending, events],
  );
  const busy = visiblePending.some((p) => p.phase === "sending" || p.phase === "acked");

  return useMemo(
    () => ({
      pending: visiblePending,
      storageKey,
      recoveryError,
      busy,
      submit,
      retry,
      discard,
      stop,
    }),
    [visiblePending, storageKey, recoveryError, busy, submit, retry, discard, stop],
  );
}
