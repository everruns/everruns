"use client";

// Global chat session management.
// Uses POST /v1/sessions/chat for per-user singleton get-or-create.
// The backend manages the Platform Chat harness and user-scoped tags.

import { useCallback, useEffect, useRef, useState } from "react";
import { useOrg } from "@/providers/org-provider";
import { getOrCreateChatSession } from "@/lib/api/sessions";
import { useLocale } from "@/providers/locale-provider";

interface UseGlobalChatResult {
  sessionId: string | null;
  isLoading: boolean;
  error: Error | null;
}

export function useGlobalChat(): UseGlobalChatResult {
  const { currentOrg } = useOrg();
  const { backendLocale } = useLocale();
  const orgId = currentOrg?.public_id;

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);
  const initRef = useRef(false);
  const currentOrgRef = useRef<string | undefined>(undefined);

  const initSession = useCallback(async () => {
    setIsLoading(true);
    setError(null);

    try {
      const session = await getOrCreateChatSession(backendLocale);
      setSessionId(session.id);
    } catch (e) {
      setError(e instanceof Error ? e : new Error("Failed to initialize global chat"));
    } finally {
      setIsLoading(false);
    }
  }, [backendLocale]);

  useEffect(() => {
    if (!orgId) return;

    // Reset when org changes
    if (currentOrgRef.current !== orgId) {
      currentOrgRef.current = orgId;
      initRef.current = false;
      setSessionId(null);
    }

    if (initRef.current) return;
    initRef.current = true;

    initSession();
  }, [orgId, initSession]);

  return { sessionId, isLoading, error };
}
