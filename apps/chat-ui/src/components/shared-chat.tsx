"use client";
import { useEffect, useRef, useState } from "react";
import { MessageContent } from "./message-content";
import { conversationMessages, originLabel, type ChatEvent } from "./transcript";
export function SharedChat({ token, name }: { token: string; name: string }) {
  const [events, setEvents] = useState<ChatEvent[]>([]),
    [title, setTitle] = useState(name),
    [before, setBefore] = useState<number | null>(null),
    [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const olderLoaded = useRef(false);
  async function load(cursor?: number) {
    setLoading(true);
    try {
      const response = await fetch(
        `/api/chat/shared/${encodeURIComponent(token)}${cursor ? `?before=${cursor}` : ""}`,
        { cache: "no-store" },
      );
      if (!response.ok)
        throw new Error("This shared conversation is unavailable or its link was revoked.");
      const page = await response.json();
      setTitle(page.title || name);
      setError("");
      if (cursor) olderLoaded.current = true;
      if (cursor || !olderLoaded.current) setBefore(page.before ?? null);
      setEvents((old) => {
        const merged = new Map(
          [...old, ...page.data].map((event: ChatEvent) => [event.id ?? event.sequence, event]),
        );
        return [...merged.values()].sort((a, b) => (a.sequence ?? 0) - (b.sequence ?? 0));
      });
    } catch (error) {
      olderLoaded.current = false;
      setBefore(null);
      setEvents([]);
      setError(error instanceof Error ? error.message : "Conversation unavailable");
    } finally {
      setLoading(false);
    }
  }
  useEffect(() => {
    void load();
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, 15000);
    return () => clearInterval(timer);
  }, [token]);
  return (
    <main className="shared-chat">
      <header>
        <div>
          <strong>{title}</strong>
          <small>Public · Read-only</small>
        </div>
      </header>
      <section className="conversation">
        {error && <p role="alert">{error}</p>}
        {before && (
          <button disabled={loading} onClick={() => load(before)}>
            Older messages
          </button>
        )}
        {conversationMessages(events).map((message) =>
          message.origin ? (
            <p key={message.id} className="automatic">
              {originLabel(message.origin)}
            </p>
          ) : (
            <article
              key={message.id}
              className={`message ${message.role === "user" ? "user" : "assistant"}`}
            >
              <MessageContent text={message.text} />
            </article>
          ),
        )}
        {loading && <p role="status">Loading conversation…</p>}
      </section>
      <footer>Shared from {name} · Work Chat · Very large messages may be omitted</footer>
    </main>
  );
}
