/** Platform conversations are ordinary sessions on the managed Agent. */

import type { Session } from "@/lib/api/types";

/** Tag written on every thread created from the Chats surface. */
export const CHAT_THREAD_TAG = "chat";

/** Marks the one automatically created Platform Chat per owner and org. The
 *  database enforces uniqueness for this tag, including archived sessions. */
export const PLATFORM_CHAT_STARTER_TAG = "platform-chat-starter";

/** Tag written by the retired `POST /v1/sessions/chat` singleton. Threads that
 *  predate the Chats surface still carry it, so they stay visible. */
export const LEGACY_GLOBAL_CHAT_TAG = "global-chat";

/** Managed Agent used by every platform conversation. */
export const PLATFORM_CHAT_AGENT_NAME = "platform-chat";

/** Legacy name retained for historical harness references. */
export const PLATFORM_CHAT_HARNESS_NAME = "platform-chat";

/** Most-recently-active threads shown under the sidebar's Chats entry. The cap
 *  is the point: live threads in the nav means the nav is never the same twice,
 *  so it must be bounded and stable. */
export const SIDEBAR_THREAD_LIMIT = 5;

/** Bounded sidebar query size; source, Agent, and owner filtering happen on the server. */
export const THREAD_SCAN_LIMIT = 100;

export function isChatThread(session: Session): boolean {
  return (
    session.source !== "playground" &&
    (session.tags.includes(CHAT_THREAD_TAG) || session.tags.includes(LEGACY_GLOBAL_CHAT_TAG))
  );
}

/** Last activity for ordering: the most recent of the session's timestamps.
 *  `updated_at` moves on every turn, so it carries the ordering on its own in
 *  practice; the max keeps ordering sane for rows a backfill left behind. */
export function threadActivityAt(session: Session): number {
  const candidates = [
    session.updated_at,
    session.finished_at,
    session.started_at,
    session.created_at,
  ];
  let latest = 0;
  for (const candidate of candidates) {
    if (!candidate) continue;
    const parsed = Date.parse(candidate);
    if (!Number.isNaN(parsed) && parsed > latest) latest = parsed;
  }
  return latest;
}

/** An archived thread is one that was explicitly put away. It keeps its
 *  transcript and its URL; it just stops competing for attention. */
export function isArchivedThread(session: Pick<Session, "archived_at">): boolean {
  return !!session.archived_at;
}

/** Sort the server-authorized thread page. Runtime ownership is checked by the server. */
export function selectChatThreads(
  sessions: Session[],
  options: { includeArchived?: boolean } = {},
): Session[] {
  return sessions
    .filter(isChatThread)
    .filter((session) => options.includeArchived || !isArchivedThread(session))
    .sort((a, b) => {
      const pinOrder = Number(b.is_pinned === true) - Number(a.is_pinned === true);
      if (pinOrder) return pinOrder;
      const archiveOrder = Number(isArchivedThread(a)) - Number(isArchivedThread(b));
      return archiveOrder || threadActivityAt(b) - threadActivityAt(a);
    });
}

/** Title to show for a thread that has not been named yet. */
export function threadTitle(
  session: Pick<Session, "title" | "preview">,
  untitled = "New chat",
): string {
  const title = session.title?.trim();
  if (title) return title;
  const preview = session.preview?.trim();
  if (preview) return preview.length > 60 ? `${preview.slice(0, 60)}…` : preview;
  return untitled;
}
