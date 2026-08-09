"use client";

// Saved sessions views, persisted per user (EVE-853).
//
// Backed by the existing user-preference store rather than a new API: a view is
// only a name and a query string, and preferences already give us per-user,
// cross-device persistence with cache invalidation.

import { useCallback, useMemo } from "react";
import { useSetUserPreference, useUserPreference } from "./use-user-preferences";
import {
  DEFAULT_SESSION_VIEWS,
  SESSION_VIEWS_PREFERENCE_KEY,
  parseSavedViews,
  type SavedSessionView,
} from "@/lib/session-views";

export interface UseSessionViewsResult {
  /** The user's saved views, or the suggested defaults when they have none. */
  views: SavedSessionView[];
  /** False while `views` holds the suggested defaults, which nobody owns. */
  viewsAreSaved: boolean;
  /** True while the stored preference is still loading. */
  isLoading: boolean;
  /** True while a save or removal is in flight. */
  isSaving: boolean;
  /** Save the given query string under a name. Replaces a view of the same name. */
  saveView: (name: string, query: string) => Promise<void>;
  /** Remove a saved view. Built-in suggestions cannot be removed. */
  removeView: (id: string) => Promise<void>;
}

export function useSessionViews(): UseSessionViewsResult {
  const preference = useUserPreference(SESSION_VIEWS_PREFERENCE_KEY);
  const setPreference = useSetUserPreference();

  const saved = useMemo(() => parseSavedViews(preference.data?.value), [preference.data?.value]);
  // Suggestions stand in only until the user has views of their own; mixing the
  // two would make a "delete" that silently fails on half the list.
  const views = saved.length > 0 ? saved : DEFAULT_SESSION_VIEWS;

  const persist = useCallback(
    async (next: SavedSessionView[]) => {
      await setPreference.mutateAsync({ key: SESSION_VIEWS_PREFERENCE_KEY, value: next });
    },
    [setPreference],
  );

  const saveView = useCallback(
    async (name: string, query: string) => {
      const trimmed = name.trim();
      if (!trimmed) return;
      const withoutDuplicate = saved.filter(
        (view) => view.name.toLowerCase() !== trimmed.toLowerCase(),
      );
      await persist([
        ...withoutDuplicate,
        { id: `view:${trimmed.toLowerCase()}:${query}`, name: trimmed, query },
      ]);
    },
    [persist, saved],
  );

  const removeView = useCallback(
    async (id: string) => {
      if (!saved.some((view) => view.id === id)) return;
      await persist(saved.filter((view) => view.id !== id));
    },
    [persist, saved],
  );

  return {
    views,
    viewsAreSaved: saved.length > 0,
    isLoading: preference.isLoading,
    isSaving: setPreference.isPending,
    saveView,
    removeView,
  };
}
