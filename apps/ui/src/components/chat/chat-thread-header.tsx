/**
 * Thread header: who you are talking to, what the thread is called, and the two
 * ways out of it.
 *
 * The agent binding is shown as a fact, not a control — a thread's transcript is
 * only meaningful against the agent that produced it, so switching agents starts
 * a new thread (knowledge/ui/information-architecture.md).
 *
 * "Share" copies the thread URL. A thread is an ordinary session, so access is
 * the session's access: the link works for people who can already see the org's
 * sessions, and mints nothing.
 */
"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import Link from "next/link";
import { Check, Copy, ExternalLink, Pencil } from "lucide-react";
import type { Session } from "@/lib/api/types";
import { AgentAvatar } from "@/components/chat/agent-avatar";
import { ChatPinButton } from "@/components/chat/chat-pin-button";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useUpdateSession } from "@/hooks/use-sessions";

function ThreadTitle({ session, title }: { session: Session; title: string }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(session.title ?? "");
  const inputRef = useRef<HTMLInputElement>(null);
  const updateSession = useUpdateSession();

  useEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing]);

  const commit = () => {
    const next = value.trim();
    setEditing(false);
    if (!next || next === (session.title ?? "").trim()) return;
    updateSession.mutate({ sessionId: session.id, request: { title: next } });
  };

  if (editing) {
    return (
      <Input
        ref={inputRef}
        value={value}
        aria-label="Thread title"
        className="h-8 max-w-sm"
        onChange={(event) => setValue(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
          if (event.key === "Escape") {
            setValue(session.title ?? "");
            setEditing(false);
          }
        }}
      />
    );
  }

  return (
    <button
      type="button"
      className="group inline-flex min-w-0 items-center gap-1.5 text-left"
      onClick={() => {
        setValue(session.title ?? "");
        setEditing(true);
      }}
      aria-label="Rename thread"
    >
      <span className="truncate text-base font-semibold tracking-tight">{title}</span>
      <Pencil className="size-3.5 flex-none text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" />
    </button>
  );
}

function ShareButton() {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);

  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      className="max-sm:w-7 max-sm:px-0"
      aria-label={copied ? "Link copied" : "Share"}
      onClick={async () => {
        await navigator.clipboard?.writeText(window.location.href);
        setCopied(true);
      }}
    >
      {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
      <span className="max-sm:hidden">{copied ? "Link copied" : "Share"}</span>
    </Button>
  );
}

export function ChatThreadHeader({
  session,
  title,
  counterpart,
  counterpartHref,
}: {
  session: Session;
  /** Display title, already resolved through the thread-title fallbacks. */
  title: string;
  /** Display name of the agent (or harness) this thread is bound to. */
  counterpart?: string;
  /** Optional linked rendering of the counterpart, e.g. through to the agent. */
  counterpartHref?: ReactNode;
}) {
  return (
    <div className="flex items-center gap-3 border-b border-border/70 bg-background/70 px-4 py-3 backdrop-blur-[1px] sm:px-6">
      <AgentAvatar name={counterpart} />
      <div className="flex min-w-0 flex-col">
        <ThreadTitle session={session} title={title} />
        <span className="truncate text-xs text-muted-foreground">
          {counterpartHref ?? counterpart ?? "No agent bound"}
        </span>
      </div>
      <div className="ml-auto flex items-center gap-2">
        <ChatPinButton session={session} showLabel />
        <ShareButton />
        <Link
          href={`/sessions/${session.id}/transcript`}
          aria-label="Open session"
          className="inline-flex items-center gap-1.5 border border-border/70 px-2.5 py-1.5 text-[13px] font-medium transition-colors hover:bg-muted/40 max-sm:size-7 max-sm:justify-center max-sm:px-0"
        >
          <ExternalLink className="size-3.5" />
          <span className="max-sm:hidden">Open session</span>
        </Link>
      </div>
    </div>
  );
}
