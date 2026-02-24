"use client";

// Global chat session management.
// Lazily creates one session per org, persists session ID in localStorage.
// Uses the Generic harness (seed ID) for the session.

import { useCallback, useEffect, useRef, useState } from "react";
import { useOrg } from "@/providers/org-provider";
import { createSession, getSession } from "@/lib/api/sessions";

const GENERIC_HARNESS_ID = "harness_01933b5a000070008000000000000602";
const STORAGE_KEY_PREFIX = "everruns:global-chat:session:";

function getStorageKey(orgId: string): string {
  return `${STORAGE_KEY_PREFIX}${orgId}`;
}

interface UseGlobalChatResult {
  sessionId: string | null;
  isLoading: boolean;
  error: Error | null;
}

export function useGlobalChat(): UseGlobalChatResult {
  const { currentOrg } = useOrg();
  const orgId = currentOrg?.public_id;

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);
  const initRef = useRef(false);
  const currentOrgRef = useRef<string | undefined>(undefined);

  const initSession = useCallback(async (org: string) => {
    setIsLoading(true);
    setError(null);

    try {
      // Check localStorage for existing session
      const storageKey = getStorageKey(org);
      const storedId = window.localStorage.getItem(storageKey);

      if (storedId) {
        // Verify the session still exists
        try {
          await getSession(storedId);
          setSessionId(storedId);
          setIsLoading(false);
          return;
        } catch {
          // Session was deleted or inaccessible — create a new one
          window.localStorage.removeItem(storageKey);
        }
      }

      // Create a new global chat session
      const session = await createSession({
        harness_id: GENERIC_HARNESS_ID,
        title: "Chat",
      });
      window.localStorage.setItem(storageKey, session.id);
      setSessionId(session.id);
    } catch (e) {
      setError(e instanceof Error ? e : new Error("Failed to initialize global chat"));
    } finally {
      setIsLoading(false);
    }
  }, []);

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

    initSession(orgId);
  }, [orgId, initSession]);

  return { sessionId, isLoading, error };
}
