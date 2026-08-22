/**
 * Chat thread vocabulary.
 *
 * A chat thread is an ordinary session: created with `POST /v1/sessions` bound
 * to an agent, renamed with `PATCH /v1/sessions/{id}`, listed with
 * `GET /v1/sessions`. There is no chat-specific API.
 *
 * Marking a session as a chat thread is the one thing the API cannot express
 * yet. EVE-852 adds a typed `source` on the session plus `source=chat` + `mine`
 * filters on the list endpoint; until then a thread is marked with a tag and
 * recognised client-side. When that lands, delete `isChatThread` and
 * `selectChatThreads` and pass the filters to the server instead — nothing else
 * in this module changes.
 */

import type { Session } from "@/lib/api/types";

/** Tag written on every thread created from the Chats surface. */
export const CHAT_THREAD_TAG = "chat";

/** Tag written by the retired `POST /v1/sessions/chat` singleton. Threads that
 *  predate the Chats surface still carry it, so they stay visible. */
export const LEGACY_GLOBAL_CHAT_TAG = "global-chat";

/** Built-in operator chat harness. A thread bound to it has no agent. */
export const PLATFORM_CHAT_HARNESS_NAME = "platform-chat";

/** Most-recently-active threads shown under the sidebar's Chats entry. The cap
 *  is the point: live threads in the nav means the nav is never the same twice,
 *  so it must be bounded and stable. */
export const SIDEBAR_THREAD_LIMIT = 5;

/** How many sessions to scan for threads while the server cannot filter (EVE-852). */
export const THREAD_SCAN_LIMIT = 100;

export function isChatThread(session: Session): boolean {
  return session.tags.includes(CHAT_THREAD_TAG) || session.tags.includes(LEGACY_GLOBAL_CHAT_TAG);
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

/**
 * Pick this user's chat threads out of a session page. Pinned threads come
 * first and archived ones last, with each group ordered by most recent
 * activity. `userId` is undefined when auth is off (local development, single
 * anonymous user) — every thread is then "mine".
 *
 * Archived threads are dropped unless `includeArchived` is set. The server
 * applies the same default to the query, so this filter only matters for a
 * page that asked for archived rows and then wants to hide them again.
 */
export function selectChatThreads(
  sessions: Session[],
  userId?: string,
  options: { includeArchived?: boolean } = {},
): Session[] {
  return sessions
    .filter(isChatThread)
    .filter((session) => options.includeArchived || !isArchivedThread(session))
    .filter(
      (session) =>
        !userId || !session.resolved_owner_user_id || session.resolved_owner_user_id === userId,
    )
    .sort((a, b) => {
      const pinOrder = Number(b.is_pinned === true) - Number(a.is_pinned === true);
      if (pinOrder) return pinOrder;
      const archiveOrder = Number(isArchivedThread(a)) - Number(isArchivedThread(b));
      return archiveOrder || threadActivityAt(b) - threadActivityAt(a);
    });
}

/** Title to show for a thread that has not been named yet. */
export function threadTitle(session: Pick<Session, "title" | "preview">): string {
  const title = session.title?.trim();
  if (title) return title;
  const preview = session.preview?.trim();
  if (preview) return preview.length > 60 ? `${preview.slice(0, 60)}…` : preview;
  return "New chat";
}
