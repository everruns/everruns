"use client";

// AgentID sign-in for the Public Chat surface. The button starts a server-side
// AgentID login for this channel; the callback returns here with a short
// runtime token in the URL fragment (never sent to a server), which this hook
// moves into session storage and clears from the address bar. The token is
// the channel's runtime credential: it expires in minutes and a new sign-in
// renews it. See knowledge/integrations/agentid.md.

import { useEffect, useState } from "react";

const ERRORS: Record<string, string> = {
  denied: "AgentID sign-in was cancelled.",
  owner_limit: "This agent's owner has reached the agent limit for this chat.",
  owner_required: "AgentID did not share the agent's owner, so sign-in could not finish.",
  unavailable: "AgentID sign-in is not available for this chat.",
  failed: "AgentID sign-in failed. Try again.",
};

interface StoredSession {
  token: string;
  expiresAt: number;
}

function storageKey(appId: string): string {
  return `everruns_public_chat_agentid:${appId}`;
}

function readStored(appId: string): StoredSession | null {
  try {
    const raw = window.sessionStorage.getItem(storageKey(appId));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as StoredSession;
    return parsed.expiresAt > Date.now() ? parsed : null;
  } catch {
    return null;
  }
}

export function agentIdLoginUrl(appId: string): string {
  return `/api/v1/channels/${encodeURIComponent(appId)}/public-chat/agentid/login`;
}

/** The visitor's AgentID runtime token for this chat, if signed in. */
export function useAgentIdSession(appId: string): {
  token: string | null;
  error: string | null;
} {
  const [session, setSession] = useState<StoredSession | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const params = new URLSearchParams(window.location.hash.slice(1));
    const token = params.get("agentid_token");
    const failure = params.get("agentid_error");
    if (token || failure) {
      // Drop the fragment so the token does not linger in the address bar.
      window.history.replaceState(null, "", window.location.pathname + window.location.search);
    }
    if (token) {
      const seconds = Number(params.get("expires_in") ?? "0") || 0;
      const fresh = { token, expiresAt: Date.now() + seconds * 1000 };
      try {
        window.sessionStorage.setItem(storageKey(appId), JSON.stringify(fresh));
      } catch {
        // Storage can be unavailable; the session still lasts for this page.
      }
      setSession(fresh);
      return;
    }
    if (failure) setError(ERRORS[failure] ?? ERRORS.failed);
    setSession(readStored(appId));
  }, [appId]);

  // Forget the token when it expires so the page offers sign-in again.
  useEffect(() => {
    if (!session) return;
    const timer = window.setTimeout(
      () => setSession(null),
      Math.max(0, session.expiresAt - Date.now()),
    );
    return () => window.clearTimeout(timer);
  }, [session]);

  return { token: session?.token ?? null, error };
}

export function AgentIdSignInButton({ appId }: { appId: string }) {
  return (
    <a
      href={agentIdLoginUrl(appId)}
      className="flex w-full items-center justify-center rounded border border-input bg-background px-3 py-2 text-sm font-medium text-foreground hover:bg-muted"
    >
      Continue with AgentID
    </a>
  );
}
