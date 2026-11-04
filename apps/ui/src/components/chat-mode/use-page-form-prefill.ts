"use client";

import { useLayoutEffect, useRef } from "react";
import { usePageChatMode } from "./chat-mode-context";

// Synchronize a clean form on refetch, retaining dirty work for the adapter's
// baseline conflict guard. Only pass the form's explicit non-secret projection.
export function usePageFormPrefill<T>(options: {
  identity: string;
  open: boolean;
  values: T;
  saved: T;
  apply: (values: T) => void;
}) {
  const adopted = !!usePageChatMode();
  const previous = useRef<{ identity: string; open: boolean; saved: string } | null>(null);
  useLayoutEffect(() => {
    const saved = JSON.stringify(options.saved);
    const values = JSON.stringify(options.values);
    const last = previous.current;
    const opening = !last?.open || last.identity !== options.identity;
    if (adopted && options.open && values !== saved && (opening || values === last?.saved)) {
      options.apply(options.saved);
    }
    previous.current = { identity: options.identity, open: options.open, saved };
  });
}
