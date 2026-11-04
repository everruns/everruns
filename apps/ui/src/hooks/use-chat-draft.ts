"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type SetStateAction } from "react";
import { chatRecoveryEpoch, readChatDraft, writeChatRecovery } from "@/lib/chat-recovery";

export function useChatDraft(key?: string) {
  const [state, setState] = useState({ key, text: "" });
  const [error, setError] = useState(false);
  const current = useRef("");
  const recoveryEpoch = useMemo(() => chatRecoveryEpoch(), [key]);
  const activeKey = useRef(key);
  activeKey.current = key;
  useEffect(() => {
    if (!key) {
      current.current = "";
      return;
    }
    let text = "";
    try {
      text = readChatDraft(key);
      setError(false);
    } catch {
      setError(true);
    }
    current.current = text;
    setState({ key, text });
  }, [key]);
  const setText = useCallback(
    (value: SetStateAction<string>) => {
      if (activeKey.current !== key) return;
      const text = typeof value === "function" ? value(current.current) : value;
      current.current = text;
      setState({ key, text });
      try {
        writeChatRecovery(key, { draft: text }, recoveryEpoch);
        setError(false);
      } catch {
        setError(true);
      }
    },
    [key, recoveryEpoch],
  );
  return [state.key === key ? state.text : "", setText, error] as const;
}
