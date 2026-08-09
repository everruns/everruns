// Saved sessions views (EVE-853).
//
// A view is a name plus the filter query string — nothing more. Because the
// sessions page keeps its whole filter set in the URL, a saved view is just a
// link: shareable by copy-paste, and reproducible for anyone who opens it.
// Persistence is the per-user preference store, so views follow a user across
// devices without needing a new API surface.

import {
  parseSessionFilters,
  serializeSessionFilters,
  type SessionFilters,
} from "@/lib/session-filters";

/** Preference key holding this user's saved sessions views. */
export const SESSION_VIEWS_PREFERENCE_KEY = "sessions.saved_views";

export interface SavedSessionView {
  /** Stable id used as the React key and for removal. */
  id: string;
  /** User-supplied name shown on the rail. */
  name: string;
  /** Serialized filter set (the sessions page query string). */
  query: string;
}

/**
 * Views offered before a user has saved any of their own. They are ordinary
 * views — selecting one only sets the URL — so the rail behaves identically
 * whether a view was suggested or saved.
 */
export const DEFAULT_SESSION_VIEWS: SavedSessionView[] = [
  { id: "builtin:failures-7d", name: "Failures, last 7 days", query: "status=failed&window=7d" },
  { id: "builtin:running", name: "Running now", query: "status=running" },
  {
    id: "builtin:scheduled-24h",
    name: "Scheduled runs, last 24h",
    query: "source=schedule&window=24h",
  },
];

/** Parse the stored preference value, tolerating anything unexpected. */
export function parseSavedViews(value: unknown): SavedSessionView[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry) => {
    if (typeof entry !== "object" || entry === null) return [];
    const { id, name, query } = entry as Record<string, unknown>;
    if (typeof id !== "string" || typeof name !== "string" || typeof query !== "string") return [];
    return [{ id, name, query }];
  });
}

/**
 * Whether a view describes the filters currently on screen. Compared on the
 * canonical serialization with pagination dropped, so "page 2 of Failures" is
 * still recognizably the Failures view.
 */
export function isViewActive(view: SavedSessionView, filters: SessionFilters): boolean {
  return normalizeViewQuery(view.query) === serializeSessionFilters({ ...filters, page: 0 });
}

/**
 * Round-trip a stored query string through the filter parser so views saved by
 * an older build (or hand-edited) compare against today's canonical form.
 */
export function normalizeViewQuery(query: string): string {
  return serializeSessionFilters({ ...parseSessionFilters(new URLSearchParams(query)), page: 0 });
}
