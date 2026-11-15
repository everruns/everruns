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

function readLocal(key: string): string | undefined {
  try {
    return window.localStorage.getItem(key) ?? undefined;
  } catch {
    return undefined;
  }
}

function writeLocal(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Private mode or disabled storage: the server value still applies.
  }
}

const noSubscribe = () => () => {};

/**
 * One server-side choice mirrored on the device. `parse` maps anything stored
 * (including values from older builds) onto a known option.
 */
function useStoredChoice<T extends string>(
  key: string,
  localKey: string,
  parse: (value: unknown) => T,
  fallback: T,
) {
  const query = useUserPreference(key);
  const save = useSetUserPreference();
  const queryClient = useQueryClient();
  const readDevice = useCallback(() => {
    const value = readLocal(localKey);
    return value ? parse(value) : undefined;
  }, [localKey, parse]);
  const local = useSyncExternalStore(noSubscribe, readDevice, () => undefined);
  const stored = query.data ? parse(query.data.value) : undefined;
  useEffect(() => {
    if (stored) writeLocal(localKey, stored);
  }, [localKey, stored]);
  const value: T = stored ?? local ?? fallback;

  const setValue = useCallback(
    (next: T) => {
      writeLocal(localKey, next);
      const queryKey = queryKeys.userPreferences.detail(key);
      // A read still in flight would land after this and switch the choice back.
      void queryClient.cancelQueries({ queryKey });
      // Switch immediately; the request only persists the choice.
      queryClient.setQueryData(queryKey, {
        key,
        value: next,
        updated_at: new Date().toISOString(),
      });
      save.mutate({ key, value: next });
    },
    [key, localKey, queryClient, save],
  );

  // Known once the server answered, or from this device's last value.
  const ready = !!stored || !!local || !query.isLoading;
  return { value, ready, setValue, saving: save.isPending, error: save.error };
}

export function useLayoutPreference() {
  const { value, setValue, ...rest } = useStoredChoice(
    LAYOUT_PREFERENCE_KEY,
    LOCAL_KEY,
    parseAppLayout,
    "classic",
  );
  return { layout: value, setLayout: setValue, ...rest };
}

/**
 * How Chat opens in the Classic layout (decided 2026-11-15):
 * - `full_screen` (default): Chat replaces the page at `/chats`, as before.
 * - `floating`: a movable, resizable card over the page; a Chat button reopens it.
 * - `docked`: a column on the right of the page with a visible divider.
 * Floating and docked keep Chat page-aware, like Chat first. Full screen stays
 * one click away from both.
 */
export type ClassicChat = "full_screen" | "floating" | "docked";

export const CLASSIC_CHAT_PREFERENCE_KEY = "ui.classic_chat";
const CLASSIC_CHAT_LOCAL_KEY = "everruns.ui.classic_chat";

export function parseClassicChat(value: unknown): ClassicChat {
  return value === "floating" || value === "docked" ? value : "full_screen";
}

export function useClassicChatPreference() {
  const { value, setValue, ...rest } = useStoredChoice(
    CLASSIC_CHAT_PREFERENCE_KEY,
    CLASSIC_CHAT_LOCAL_KEY,
    parseClassicChat,
    "full_screen",
  );
  return { classicChat: value, setClassicChat: setValue, ...rest };
}
