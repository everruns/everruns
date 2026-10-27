"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { createContext, useContext, type ReactNode } from "react";
import { X } from "lucide-react";
import type { ChatChanges } from "./use-agent-chat-changes";
import { cn } from "@/lib/utils";
import { useChatMode } from "./chat-mode-context";
import type { DraftChatChanges } from "./use-agent-chat-draft";
import { PageChangeDiff } from "./page-change-diff";

export const ChatDraftChangesContext = createContext<DraftChatChanges | null>(null);

export const ChatChangesContext = createContext<ChatChanges | null>(null);
export const useChatChanges = () => useContext(ChatChangesContext);

export function ChatChangeMark({
  field,
  children,
  className,
}: {
  field: string | string[];
  children: ReactNode;
  className?: string;
}) {
  const ui = useUiTranslations();
  const changes = useChatChanges();
  const mode = useChatMode();
  const draft = useContext(ChatDraftChangesContext);
  const names = Array.isArray(field) ? field : [field];
  const changed = changes?.fields.filter((diff) => names.includes(diff.field)) ?? [];
  const draftChanged = draft?.fields.filter((diff) => names.includes(diff.field)) ?? [];
  if (!changed.length && !draftChanged.length)
    return <div className={cn("min-w-0", className)}>{children}</div>;
  return (
    <div
      className={cn("min-w-0 border-l-2 border-accent bg-accent/[0.08] pl-3", className)}
      data-chat-changed={[...changed, ...draftChanged].map((diff) => diff.field).join(" ")}
    >
      {children}
      {changed.map((diff) => (
        <p key={diff.field} className="py-1 text-[11px] text-muted-foreground">
          {ui("Changed by chat ·")}{" "}
          <button
            type="button"
            disabled={changes?.pending || mode?.page.editing || changes?.canUndo === false}
            onClick={() => void changes?.undo(diff.field)}
            className="underline underline-offset-2 disabled:opacity-50"
          >
            {ui("Undo")}
            <span className="sr-only"> {diff.field}</span>
          </button>
        </p>
      ))}
      {draftChanged.map((diff) => (
        <p key={`draft-${diff.field}`} className="py-1 text-[11px] text-muted-foreground">
          {ui("Draft edited by Chat ·")}{" "}
          <button
            type="button"
            className="underline underline-offset-2"
            onClick={() => draft?.undo(diff.field)}
          >
            {ui("Undo")}
            <span className="sr-only">
              {" "}
              {ui("draft")} {diff.field}
            </span>
          </button>
        </p>
      ))}
    </div>
  );
}

export function ChatDraftChangesToast() {
  const ui = useUiTranslations();
  const draft = useContext(ChatDraftChangesContext);
  if (!draft || (!draft.fields.length && !draft.error)) return null;
  return (
    <div
      role="status"
      className="sticky bottom-3 z-10 mx-3 mt-3 border border-accent bg-card p-3 shadow-sm"
    >
      <div className="flex items-center gap-3 text-sm">
        <span className="flex-1">{ui("Chat edited your draft. Save changes to apply it.")}</span>
        <button type="button" className="text-xs underline" onClick={() => draft.undo()}>
          {ui("Undo draft edits")}
        </button>
        <button type="button" aria-label={ui("Dismiss draft change markers")} onClick={draft.clear}>
          <X className="size-4" />
        </button>
      </div>
      <PageChangeDiff fields={draft.fields} undo={draft.undo} />
      {draft.error && (
        <p role="alert" className="mt-1 text-xs text-destructive">
          {draft.error}
        </p>
      )}
    </div>
  );
}

export function ChatChangesToast({
  name,
  canUndo,
  editing,
}: {
  name: string;
  canUndo: boolean;
  editing: boolean;
}) {
  const ui = useUiTranslations();
  const changes = useChatChanges();
  if (!changes || changes.shared || (!changes.fields.length && !changes.error)) return null;
  return (
    <div
      className="sticky bottom-3 z-10 mx-3 mt-3 border border-accent bg-card p-3 shadow-sm"
      role="status"
    >
      <div className="flex flex-wrap items-center gap-3 text-sm">
        <span className="flex-1">
          {changes.fields.length
            ? ui("Chat saved {value0} {value1} to {value2}", {
                value0: changes.fields.length,
                value1: changes.fields.length === 1 ? "change" : "changes",
                value2: name,
              })
            : ui("Chat changes")}
        </span>
        {canUndo && changes.fields.length > 0 && (
          <button
            type="button"
            onClick={() => void changes.undo()}
            disabled={changes.pending || editing}
            className="text-xs underline underline-offset-2 disabled:opacity-50"
          >
            {changes.pending ? ui("Undoing…") : ui("Undo all")}
          </button>
        )}
        <button
          type="button"
          onClick={changes.dismiss}
          aria-label={ui("Dismiss Chat change markers")}
        >
          <X className="size-4" />
        </button>
      </div>
      <PageChangeDiff
        fields={changes.fields}
        pending={changes.pending}
        canUndo={canUndo && !editing}
        undo={(field) => void changes.undo(field)}
      />
      {editing && changes.fields.length > 0 && (
        <p className="mt-1 text-xs text-muted-foreground">
          {ui(
            "Chat saved changes while you were editing. Your draft is kept. Discard it to review the saved changes before continuing.",
          )}
        </p>
      )}
      {changes.error && (
        <p role="alert" className="mt-1 text-xs text-destructive">
          {changes.error}
        </p>
      )}
    </div>
  );
}
