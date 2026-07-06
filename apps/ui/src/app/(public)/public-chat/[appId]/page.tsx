"use client";

// Isolated Public Chat page. Renders a branded, chrome-free chat bound to a
// single App's agent and talks only to the app-scoped public endpoints. No
// console navigation, no other apps/agents reachable from here.
// See specs/public-chat.md.

import { useCallback, useEffect, useRef, useState } from "react";
import { useParams } from "next/navigation";
import { Loader2, Send } from "lucide-react";

import {
  type ChatMessage,
  type PublicChatBootstrap,
  fetchBootstrap,
  runPublicChat,
} from "@/lib/public-chat/client";
import { TurnstileWidget } from "./turnstile-widget";
import { GoogleSignInButton } from "./google-signin";

function threadStorageKey(appId: string): string {
  return `everruns_public_chat_thread:${appId}`;
}

export default function PublicChatPage() {
  const params = useParams<{ appId: string }>();
  const appId = String(params.appId);

  const [bootstrap, setBootstrap] = useState<PublicChatBootstrap | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [threadId, setThreadId] = useState<string | null>(null);
  const [turnstileToken, setTurnstileToken] = useState<string | null>(null);
  // Bumped after each send to force a fresh single-use Turnstile token.
  const [captchaNonce, setCaptchaNonce] = useState(0);
  // Google ID token (JWT) once the visitor signs in.
  const [googleIdToken, setGoogleIdToken] = useState<string | null>(null);

  const scrollRef = useRef<HTMLDivElement | null>(null);

  const googleSignIn = bootstrap?.sign_in?.mode === "google_oidc" ? bootstrap.sign_in : undefined;
  const googleClientId = googleSignIn?.google_client_id;
  const signedIn = Boolean(googleIdToken);
  // When the channel configures sign-in, the server requires a verified
  // credential on every request, so sign-in is always mandatory here.
  const signInRequired = Boolean(googleSignIn);
  // Turnstile only gates the anonymous path: it never applies when sign-in is
  // configured, and signed-in visitors are exempt.
  const captchaRequired = Boolean(bootstrap?.captcha?.site_key) && !signInRequired && !signedIn;

  // Load bootstrap config + restore/create the per-browser thread id.
  useEffect(() => {
    let cancelled = false;
    fetchBootstrap(appId)
      .then((config) => {
        if (cancelled) return;
        setBootstrap(config);
        const key = threadStorageKey(appId);
        let existing = window.localStorage.getItem(key);
        if (!existing) {
          existing = crypto.randomUUID();
          window.localStorage.setItem(key, existing);
        }
        setThreadId(existing);
      })
      .catch((e: unknown) => {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : "Failed to load chat.");
      });
    return () => {
      cancelled = true;
    };
  }, [appId]);

  // Keep the latest message in view.
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [messages]);

  const handleTurnstileVerify = useCallback((token: string) => setTurnstileToken(token), []);

  const startNewConversation = useCallback(() => {
    const fresh = crypto.randomUUID();
    window.localStorage.setItem(threadStorageKey(appId), fresh);
    setThreadId(fresh);
    setMessages([]);
    setError(null);
  }, [appId]);

  const send = useCallback(async () => {
    const text = input.trim();
    if (!text || sending || !threadId) return;
    if (signInRequired && !signedIn) {
      setError("Please sign in to start chatting.");
      return;
    }
    if (captchaRequired && !turnstileToken) {
      setError("Please complete the verification below before sending.");
      return;
    }

    const userMsg: ChatMessage = { id: crypto.randomUUID(), role: "user", content: text };
    const next = [...messages, userMsg];
    setMessages(next);
    setInput("");
    setSending(true);
    setError(null);

    // Turnstile tokens are single-use; consume it and request a fresh one.
    const tokenForRun = turnstileToken ?? undefined;
    setTurnstileToken(null);
    if (captchaRequired) setCaptchaNonce((n) => n + 1);

    await runPublicChat({
      appId,
      threadId,
      messages: next,
      turnstileToken: tokenForRun,
      bearerToken: googleIdToken ?? undefined,
      callbacks: {
        onAssistantStart: (id) =>
          setMessages((cur) =>
            cur.some((m) => m.id === id) ? cur : [...cur, { id, role: "assistant", content: "" }],
          ),
        onAssistantDelta: (id, delta) =>
          setMessages((cur) => {
            if (!cur.some((m) => m.id === id)) {
              return [...cur, { id, role: "assistant", content: delta }];
            }
            return cur.map((m) => (m.id === id ? { ...m, content: m.content + delta } : m));
          }),
        onError: (msg) => setError(msg),
      },
    });

    setSending(false);
  }, [
    appId,
    threadId,
    messages,
    input,
    sending,
    captchaRequired,
    turnstileToken,
    signInRequired,
    signedIn,
    googleIdToken,
  ]);

  const handleGoogleCredential = useCallback((idToken: string) => {
    setGoogleIdToken(idToken);
    setError(null);
  }, []);

  const onComposerKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  };

  if (loadError) {
    return (
      <div className="flex min-h-screen items-center justify-center p-6 text-center">
        <p className="text-muted-foreground">{loadError}</p>
      </div>
    );
  }

  if (!bootstrap) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  const accent = bootstrap.primary_color;

  return (
    <div className="mx-auto flex h-screen max-w-3xl flex-col">
      {/* Header */}
      <header className="flex items-center gap-3 border-b border-border px-4 py-3">
        {bootstrap.logo_url ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img src={bootstrap.logo_url} alt="" className="h-8 w-8 rounded object-contain" />
        ) : null}
        <h1 className="flex-1 truncate text-base font-semibold text-foreground">
          {bootstrap.name}
        </h1>
        {messages.length > 0 ? (
          <button
            type="button"
            onClick={startNewConversation}
            className="text-xs text-muted-foreground hover:text-foreground"
          >
            New conversation
          </button>
        ) : null}
      </header>

      {/* Messages */}
      <div ref={scrollRef} className="flex-1 space-y-4 overflow-y-auto px-4 py-6">
        {messages.length === 0 && bootstrap.welcome_message ? (
          <p className="text-center text-sm text-muted-foreground">{bootstrap.welcome_message}</p>
        ) : null}
        {messages.map((m) => (
          <div key={m.id} className={m.role === "user" ? "flex justify-end" : "flex justify-start"}>
            <div
              className={
                m.role === "user"
                  ? "max-w-[85%] whitespace-pre-wrap rounded bg-primary px-3 py-2 text-sm text-primary-foreground"
                  : "max-w-[85%] whitespace-pre-wrap rounded bg-muted px-3 py-2 text-sm text-foreground"
              }
              style={m.role === "user" && accent ? { backgroundColor: accent } : undefined}
            >
              {m.content || (m.role === "assistant" && sending ? "…" : "")}
            </div>
          </div>
        ))}
      </div>

      {/* Composer */}
      <div className="border-t border-border px-4 py-3">
        {error ? <p className="mb-2 text-xs text-destructive">{error}</p> : null}
        {googleSignIn && googleClientId && !signedIn ? (
          <div className="mb-2 space-y-1">
            {signInRequired ? (
              <p className="text-center text-xs text-muted-foreground">
                Sign in to start chatting.
              </p>
            ) : null}
            <GoogleSignInButton
              clientId={googleClientId}
              onCredential={handleGoogleCredential}
              onError={() => setError("Sign-in is unavailable right now.")}
            />
          </div>
        ) : null}
        {captchaRequired && bootstrap.captcha ? (
          <div className="mb-2">
            <TurnstileWidget
              key={captchaNonce}
              siteKey={bootstrap.captcha.site_key}
              onVerify={handleTurnstileVerify}
              onExpire={() => setTurnstileToken(null)}
              onError={() => setTurnstileToken(null)}
            />
          </div>
        ) : null}
        <div className="flex items-end gap-2">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={onComposerKeyDown}
            rows={1}
            aria-label="Message"
            disabled={signInRequired && !signedIn}
            placeholder={signInRequired && !signedIn ? "Sign in to chat…" : "Type a message…"}
            className="max-h-40 min-h-[40px] flex-1 resize-none rounded border border-input bg-background px-3 py-2 text-sm text-foreground outline-none focus:ring-2 focus:ring-ring disabled:opacity-50"
          />
          <button
            type="button"
            onClick={() => void send()}
            disabled={sending || !input.trim() || (signInRequired && !signedIn)}
            className="flex h-10 w-10 items-center justify-center rounded bg-primary text-primary-foreground disabled:opacity-50"
            style={accent ? { backgroundColor: accent } : undefined}
            aria-label="Send message"
          >
            {sending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
          </button>
        </div>
      </div>
    </div>
  );
}
