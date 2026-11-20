"use client";

import dynamic from "next/dynamic";
import { useEffect, useState } from "react";
import type { AgentViewLayout, PublishedAgentView } from "@everruns/generative-ui";
import "@everruns/generative-ui/styles.css";

export const PublishedView = dynamic(
  () => import("@everruns/generative-ui").then((module) => module.PublishedView),
  { ssr: false },
);

export function useWorkChatView(enabled: boolean, session?: string | null) {
  const url = session ? "/api/chat/view?session=" + encodeURIComponent(session) : "/api/chat/view";
  const [result, setResult] = useState<{
    url: string;
    status: number;
    view: PublishedAgentView | null;
  }>();
  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    async function refresh() {
      try {
        const response = await fetch(url, { cache: "no-store", signal: controller.signal });
        const view = response.ok ? await response.json() : null;
        if (!controller.signal.aborted) setResult({ url, status: response.status, view });
      } catch {
        if (!controller.signal.aborted) setResult({ url, status: 503, view: null });
      }
    }
    void refresh();
    const timer = setInterval(() => void refresh(), 10_000);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [enabled, url]);
  const current = enabled && result?.url === url ? result : undefined;
  return {
    status: current?.status,
    view: current?.view,
    async save(revision: number, layout: AgentViewLayout | null) {
      const response = await fetch(url, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ revision, layout }),
        cache: "no-store",
      });
      if (!response.ok) {
        const problem = await response.json().catch(() => null);
        throw new Error(
          typeof problem?.detail === "string" ? problem.detail : "Could not save your layout.",
        );
      }
      const view = (await response.json()) as PublishedAgentView;
      setResult({ url, status: response.status, view });
    },
  };
}

export function PublishedLanding({ name }: { name: string }) {
  const { view, status, save } = useWorkChatView(true);
  return (
    <main className="published-landing">
      <header>
        <strong>{name}</strong>
        <a href="/?surface=chat">Chat</a>
      </header>
      {view ? (
        <PublishedView key={view.revision} view={view} saveLayout={save} />
      ) : (
        <div className="published-empty">
          {status === 401 ? (
            <a href="/auth/start">Sign in to view</a>
          ) : status === undefined ? (
            <p role="status">Loading view…</p>
          ) : (
            <p>This view is unavailable.</p>
          )}
        </div>
      )}
    </main>
  );
}
