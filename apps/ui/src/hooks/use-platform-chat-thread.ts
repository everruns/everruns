"use client";

/**
 * The user's Platform Chat thread: precreated, pinned, and the place a new
 * user lands.
 *
 * Chats is the unconditional landing route
 * (knowledge/ui/information-architecture.md), so a fresh account must find a
 * conversation already waiting there rather than an empty list and a picker.
 * The thread is an ordinary session bound to the built-in `platform-chat`
 * harness and tagged as a chat thread — nothing about it is special except
 * that the app creates it instead of the user, and pins it so it stays at the
 * top of the list.
 *
 * Ensuring is per user and per org (pins are per-user state), idempotent, and
 * one-shot per entry: if the thread already exists it is adopted, and a failed
 * attempt is simply retried the next time the app is entered. A thread the
 * user archived is *not* recreated — archiving is the user saying they do not
 * want it.
 *
 * The guard against a second thread is module-level rather than per-component,
 * because the hook can be mounted more than once at a time (the desktop and
 * mobile sidebars both render the thread list) and the created thread is only
 * visible to the others once the sessions list has refetched.
 */

import { useEffect } from "react";
import { useHarnesses } from "@/hooks";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { useCreateSession, usePinSession } from "@/hooks/use-sessions";
import { useOrg } from "@/providers/org-provider";
import { CHAT_THREAD_TAG, PLATFORM_CHAT_HARNESS_NAME } from "@/lib/chat-threads";
import type { Session } from "@/lib/api/types";

/** Title given to the precreated thread. Users may rename it afterwards; the
 *  thread is recognised by its harness binding, not by this string. */
export const PLATFORM_CHAT_THREAD_TITLE = "Platform Chat";

/** Orgs this page load has already created the thread for. Survives the
 *  remounts and parallel mounts a single page load produces; a full reload
 *  starts over, by which time the created thread is in the sessions list. */
const ensuredOrgIds = new Set<string>();

export interface UsePlatformChatThreadOptions {
  /** Create the thread when the user has none. Read-only when false. */
  ensure?: boolean;
}

export interface UsePlatformChatThreadResult {
  /** The user's Platform Chat thread, once it exists. */
  thread?: Session;
  /** True until the existing threads and harnesses have been read. */
  isLoading: boolean;
}

export function usePlatformChatThread(
  options: UsePlatformChatThreadOptions = {},
): UsePlatformChatThreadResult {
  const { ensure = false } = options;
  const { currentOrg } = useOrg();
  const orgId = currentOrg?.public_id;
  // Archived threads count as existing: a user who put the thread away must not
  // get a fresh one on the next page load.
  const { threads, isLoading: threadsLoading } = useChatThreads({
    includeArchived: true,
    poll: false,
  });
  const { data: harnesses = [], isLoading: harnessesLoading } = useHarnesses();
  const createSession = useCreateSession();
  const pinSession = usePinSession();

  const platformChat = harnesses.find((harness) => harness.name === PLATFORM_CHAT_HARNESS_NAME);
  // A thread bound straight to the harness, with no agent in between.
  const thread = platformChat
    ? threads.find((candidate) => !candidate.agent_id && candidate.harness_id === platformChat.id)
    : undefined;
  const isLoading = threadsLoading || harnessesLoading;

  useEffect(() => {
    if (!ensure || isLoading || !orgId || !platformChat || thread) return;
    if (ensuredOrgIds.has(orgId)) return;
    ensuredOrgIds.add(orgId);

    void (async () => {
      try {
        const session = await createSession.mutateAsync({
          request: {
            harness_name: PLATFORM_CHAT_HARNESS_NAME,
            title: PLATFORM_CHAT_THREAD_TITLE,
            tags: [CHAT_THREAD_TAG],
          },
        });
        await pinSession.mutateAsync({ sessionId: session.id });
      } catch {
        // Best-effort: the app works without the precreated thread, and the
        // next entry tries again.
        ensuredOrgIds.delete(orgId);
      }
    })();
    // Deliberately not keyed on the mutation objects: they are recreated on
    // every render, and the attempt guard above is what keeps this one-shot.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ensure, isLoading, orgId, platformChat, thread]);

  return { thread, isLoading };
}
