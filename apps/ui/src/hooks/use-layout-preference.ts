"use client";

import { useCallback, useEffect, useSyncExternalStore } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useSetUserPreference, useUserPreference } from "@/hooks/use-user-preferences";
import { queryKeys } from "@/lib/query-keys";

/**
 * App layout is a personal choice (knowledge/ui/chat-mode.md):
 * - `classic` (default): the left navigation sidebar; Chat opens full screen.
 * - `chat_first`: Main chat on the left, the page on the right, navigation in a right rail.
 *
 * Classic is the default (decided 2026-11-15): Chat first is opt-in until it
 * matures. Only an explicit `chat_first` choice selects it.
 *
 * Stored server-side so it follows the person across devices and organizations.
 * The last value is mirrored to localStorage so a reload renders the chosen
 * layout before the preference request returns.
 */
export type AppLayout = "chat_first" | "classic";

export const LAYOUT_PREFERENCE_KEY = "ui.layout";
const LOCAL_KEY = "everruns.ui.layout";

export function parseAppLayout(value: unknown): AppLayout {
  return value === "chat_first" ? "chat_first" : "classic";
}

function readLocal(): AppLayout | undefined {
  try {
    const value = window.localStorage.getItem(LOCAL_KEY);
    return value ? parseAppLayout(value) : undefined;
  } catch {
    return undefined;
  }
}

function writeLocal(layout: AppLayout) {
  try {
    window.localStorage.setItem(LOCAL_KEY, layout);
  } catch {
    // Private mode or disabled storage: the server value still applies.
  }
}

const noSubscribe = () => () => {};

export function useLayoutPreference() {
  const query = useUserPreference(LAYOUT_PREFERENCE_KEY);
  const save = useSetUserPreference();
  const queryClient = useQueryClient();
  const local = useSyncExternalStore(noSubscribe, readLocal, () => undefined);
  const stored = query.data ? parseAppLayout(query.data.value) : undefined;
  useEffect(() => {
    if (stored) writeLocal(stored);
  }, [stored]);
  const layout: AppLayout = stored ?? local ?? "classic";

  const setLayout = useCallback(
    (next: AppLayout) => {
      writeLocal(next);
      const queryKey = queryKeys.userPreferences.detail(LAYOUT_PREFERENCE_KEY);
      // A read still in flight would land after this and switch the layout back.
      void queryClient.cancelQueries({ queryKey });
      // Switch immediately; the request only persists the choice.
      queryClient.setQueryData(queryKey, {
        key: LAYOUT_PREFERENCE_KEY,
        value: next,
        updated_at: new Date().toISOString(),
      });
      save.mutate({ key: LAYOUT_PREFERENCE_KEY, value: next });
    },
    [queryClient, save],
  );

  // Known once the server answered, or from this device's last value.
  const ready = !!stored || !!local || !query.isLoading;
  return { layout, ready, setLayout, saving: save.isPending, error: save.error };
}
