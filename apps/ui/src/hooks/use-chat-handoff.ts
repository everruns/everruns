"use client";

import { useEffect, type RefObject, type SetStateAction } from "react";
import { CHAT_HANDOFF_EVENT, takeChatHandoff } from "@/lib/chat-handoff";

export function useChatHandoff(
  key: string | undefined,
  setDraft: (value: SetStateAction<string>) => void,
  textarea: RefObject<HTMLTextAreaElement | null>,
) {
  useEffect(() => {
    let focus: number | undefined;
    const receive = () => {
      const text = takeChatHandoff(key);
      if (!text) return;
      setDraft((current) => (current.trim() ? `${current}\n\n${text}` : text));
      focus = requestAnimationFrame(() => textarea.current?.focus());
    };
    window.addEventListener(CHAT_HANDOFF_EVENT, receive);
    receive();
    return () => {
      window.removeEventListener(CHAT_HANDOFF_EVENT, receive);
      if (focus !== undefined) cancelAnimationFrame(focus);
    };
  }, [key, setDraft, textarea]);
}
