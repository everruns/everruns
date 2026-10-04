/**
 * Thread header: who you are talking to, what the thread is called, and the two
 * ways out of it.
 *
 * The agent binding is shown as a fact, not a control — a thread's transcript is
 * only meaningful against the agent that produced it, so switching agents starts
 * a new thread (knowledge/ui/information-architecture.md).
 */
"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { ExternalLink, MessageCircle, Pencil } from "lucide-react";
import { PLATFORM_CHAT_STARTER_TAG } from "@/lib/chat-threads";
import type { Session } from "@/lib/api/types";
import { PageMasthead } from "@/components/layout/page-layout";
import { cn } from "@/lib/utils";
import { AgentAvatar } from "@/components/chat/agent-avatar";
import { ChatArchiveButton } from "@/components/chat/chat-archive-button";
import { ChatPinButton } from "@/components/chat/chat-pin-button";
import { LinkButton } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useUpdateSession } from "@/hooks/use-sessions";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";

function ThreadTitle({
  session,
  title,
  large = false,
}: {
  session: Session;
  title: string;
  large?: boolean;
}) {
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
      className="group inline-flex max-w-full min-w-0 items-center gap-1.5 text-left"
      onClick={() => {
        setValue(session.title ?? "");
        setEditing(true);
      }}
      aria-label="Rename thread"
    >
      <span
        className={cn(
          "min-w-0 truncate font-semibold tracking-tight",
          large ? "text-inherit" : "text-base",
        )}
      >
        {title}
      </span>
      <Pencil className="size-3.5 flex-none text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" />
    </button>
  );
}

export function ChatThreadHeader({
  session,
  title,
  counterpart,
  counterpartHref,
  platformIntro,
  platformDescription,
  contextLabel,
  layout = "chat",
  showPin = true,
  extraActions,
  threadMode = false,
}: {
  contextLabel?: ReactNode;
  layout?: "chat" | "page";
  showPin?: boolean;
  extraActions?: ReactNode;
  threadMode?: boolean;
  session: Session;
  /** Display title, already resolved through the thread-title fallbacks. */
  title: string;
  /** Display name of the agent (or harness) this thread is bound to. */
  counterpart?: string;
  /** Optional linked rendering of the counterpart, e.g. through to the agent. */
  counterpartHref?: ReactNode;
  /**
   * Platform Chat intro (Markdown). While it is visible in the intro box
   * below, the header keeps the counterpart line; once the intro hides (the
   * user inputs), the header swaps to the short description.
   */
  platformIntro?: string | null;
  /** One-line Platform Chat description in simplified Markdown (agent wins). */
  platformDescription?: string | null;
}) {
  // The intro box shows on a fresh thread; once the user inputs it hides and
  // the header takes over with the short description, animated below.
  const { chatEvents } = useSessionContext();
  const permanent = session.tags.includes(PLATFORM_CHAT_STARTER_TAG);
  const introVisible = !!platformIntro && chatEvents.length === 0;
  const showDescription = !!platformDescription && !introVisible;
  const actions = (
    <>
      {!permanent && (
        <>
          {showPin && !threadMode && <ChatPinButton session={session} showLabel />}
          <ChatArchiveButton
            session={session}
            showLabel
            resolve={threadMode}
            disabled={threadMode && !session.archived_at && session.status !== "idle"}
          />
        </>
      )}
      <LinkButton
        href={`/sessions/${session.id}/transcript`}
        variant="outline"
        size="sm"
        aria-label="Open session"
        className="max-sm:w-7 max-sm:px-0"
      >
        <ExternalLink className="size-4" />
        <span className="max-sm:hidden">Open session</span>
      </LinkButton>
      {extraActions}
    </>
  );
  if (layout === "page") {
    return (
      <PageMasthead
        icon={<MessageCircle />}
        title={<ThreadTitle session={session} title={title} large />}
        badges={
          session.archived_at ? (
            <span className="border px-1.5 text-xs text-muted-foreground">
              {threadMode ? "Resolved" : "Archived"}
            </span>
          ) : undefined
        }
        description={
          <>
            {counterpartHref ?? counterpart ?? "No agent bound"}
            {contextLabel && <> · {contextLabel}</>}
          </>
        }
        actions={actions}
        compactActions={actions}
      />
    );
  }
  return (
    <div className="flex items-center gap-3 border-b border-border/70 bg-background/70 px-4 py-3 backdrop-blur-[1px] sm:px-6">
      <AgentAvatar name={counterpart} />
      <div className="flex min-w-0 flex-col">
        <span className="flex min-w-0 items-center gap-2">
          {permanent ? (
            <span className="text-base font-semibold">Chat</span>
          ) : (
            <ThreadTitle session={session} title={title} />
          )}
          {session.archived_at && (
            <span className="flex-none border border-border/70 px-1.5 text-[11px] uppercase tracking-wide text-muted-foreground">
              {threadMode ? "Resolved" : "Archived"}
            </span>
          )}
        </span>
        <span
          className="grid transition-[grid-template-rows] duration-300 ease-out"
          style={{ gridTemplateRows: showDescription ? "1fr" : "0fr" }}
        >
          <span className="min-h-0 overflow-hidden">
            {platformDescription ? (
              <InlineStreamdownMessage className="truncate text-xs text-muted-foreground">
                {platformDescription}
              </InlineStreamdownMessage>
            ) : null}
          </span>
        </span>
        {!showDescription ? (
          <span className="truncate text-xs text-muted-foreground">
            {counterpartHref ?? counterpart ?? "No agent bound"}
            {contextLabel && <> · {contextLabel}</>}
          </span>
        ) : null}
      </div>
      <div className="ml-auto flex items-center gap-2">{actions}</div>
    </div>
  );
}
