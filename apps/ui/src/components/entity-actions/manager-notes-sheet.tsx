"use client";

// Manager notes sheet: the markdown notes an entity's managers keep about it
// (requirements, rationale, ownership). View by default, Edit to change.
//
// Decisions:
// - Save sends the revision the edit started from. When someone else saved in
//   between, the server refuses with `manager_context_changed`; the sheet says
//   so and keeps the draft, so nothing typed is lost on a conflict.
// - The reason is optional here, as everywhere a person saves.
// - The entity itself never sees its notes (they never reach its runtime),
//   which the empty state says, so a manager knows these are not instructions
//   to the agent.
// See knowledge/execution/change-reasons-and-manager-context.md (UI).

import { useState } from "react";
import { NotebookPen, Pencil } from "lucide-react";
import { ChangeReasonField } from "@/components/entity-actions/change-reason-field";
import { entityKindLabel, type EntityKind } from "@/components/entity-actions/entity-kinds";
import { Button } from "@/components/ui/button";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import { MarkdownDisplay } from "@/components/ui/prompt-editor";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { useManagerContext, useSetManagerContext } from "@/hooks/use-change-history";
import { ApiError } from "@/lib/api/client";
import { MANAGER_CONTEXT_CHANGED } from "@/lib/api/change-history";
import { formatRelativeTime } from "@/lib/formatting";

/** Largest notes document the server accepts. */
const MAX_NOTES_BYTES = 16 * 1024;

interface ManagerNotesSheetProps {
  entityRef: string;
  kind: EntityKind;
  entityName: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function isManagerContextConflict(error: unknown): boolean {
  return (
    error instanceof ApiError && (error.code === MANAGER_CONTEXT_CHANGED || error.status === 409)
  );
}

export function ManagerNotesSheet({
  entityRef,
  kind,
  entityName,
  open,
  onOpenChange,
}: ManagerNotesSheetProps) {
  const notes = useManagerContext(entityRef, open);
  const save = useSetManagerContext(entityRef);
  // While editing: the text and the revision the edit started from.
  const [draft, setDraft] = useState<{ content: string; baseRevision: number } | null>(null);
  const [reason, setReason] = useState("");
  const label = entityKindLabel(kind);
  const content = notes.data?.content ?? "";
  const bytes = draft ? new TextEncoder().encode(draft.content).length : 0;
  const conflict = isManagerContextConflict(save.error);

  const startEdit = () => {
    save.reset();
    setDraft({ content, baseRevision: notes.data?.revision ?? 0 });
  };
  const cancelEdit = () => {
    save.reset();
    setDraft(null);
    setReason("");
  };
  const submit = async () => {
    if (!draft) return;
    try {
      await save.mutateAsync({
        content: draft.content,
        expectedRevision: draft.baseRevision,
        reason,
      });
      setDraft(null);
      setReason("");
    } catch {
      // Shown below from save.error.
    }
  };
  const reloadLatest = async () => {
    await notes.refetch();
    cancelEdit();
  };

  return (
    <Drawer
      open={open}
      onOpenChange={(next) => {
        if (!next) cancelEdit();
        onOpenChange(next);
      }}
    >
      <DrawerContent className="gap-0 overflow-y-auto p-0 sm:max-w-2xl">
        <DrawerHeader className="border-b p-5 pr-12">
          <DrawerTitle>Manager notes</DrawerTitle>
          <DrawerDescription>
            What the managers of {entityName} should know before changing it. Only managers see
            these notes.
          </DrawerDescription>
        </DrawerHeader>
        <div className="flex-1 space-y-4 p-5">
          {notes.isLoading ? (
            <Skeleton className="h-40 w-full" />
          ) : notes.error ? (
            <p role="alert" className="text-sm text-destructive">
              Could not load manager notes: {notes.error.message}
            </p>
          ) : draft ? (
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                void submit();
              }}
            >
              <Textarea
                aria-label="Manager notes"
                value={draft.content}
                onChange={(event) => setDraft({ ...draft, content: event.target.value })}
                className="min-h-64 font-mono text-sm"
                placeholder={`Requirements, rationale and ownership of this ${label}. Markdown.`}
              />
              {bytes > MAX_NOTES_BYTES && (
                <p className="text-xs text-destructive">
                  {Math.ceil(bytes / 1024)} KiB; notes are limited to 16 KiB.
                </p>
              )}
              <ChangeReasonField value={reason} onChange={setReason} />
              {conflict ? (
                <Notice variant="warning" role="alert">
                  <NoticeTitle>Someone else changed these notes</NoticeTitle>
                  <NoticeDescription className="space-y-2">
                    <span className="block">
                      They were saved again after you started editing. Copy anything you want to
                      keep, then reload the latest notes and edit again.
                    </span>
                    <Button type="button" size="sm" variant="outline" onClick={reloadLatest}>
                      Reload latest notes
                    </Button>
                  </NoticeDescription>
                </Notice>
              ) : (
                save.error && (
                  <p role="alert" className="text-sm text-destructive">
                    Could not save: {save.error.message}
                  </p>
                )
              )}
              <div className="flex justify-end gap-2">
                <Button type="button" variant="outline" onClick={cancelEdit}>
                  Cancel
                </Button>
                <Button
                  type="submit"
                  disabled={save.isPending || conflict || bytes > MAX_NOTES_BYTES}
                >
                  {save.isPending ? "Saving..." : "Save notes"}
                </Button>
              </div>
            </form>
          ) : content.trim() ? (
            <>
              <MarkdownDisplay content={content} />
              {notes.data?.updated_at && (
                <p className="text-xs text-muted-foreground">
                  Updated {formatRelativeTime(notes.data.updated_at)}
                </p>
              )}
            </>
          ) : (
            <div className="space-y-2 py-6 text-center">
              <NotebookPen className="mx-auto size-5 text-muted-foreground" />
              <p className="text-sm font-medium">No manager notes yet</p>
              <p className="mx-auto max-w-sm text-sm text-muted-foreground">
                Notes are for the people who manage this {label}: what it must keep doing, why it is
                set up this way, and who owns it. The {label} itself never sees them.
              </p>
            </div>
          )}
        </div>
        {!draft && !notes.isLoading && !notes.error && (
          <DrawerFooter className="border-t p-4">
            <Button variant="outline" onClick={startEdit}>
              <Pencil className="size-4" />
              {content.trim() ? "Edit" : "Add notes"}
            </Button>
          </DrawerFooter>
        )}
      </DrawerContent>
    </Drawer>
  );
}
