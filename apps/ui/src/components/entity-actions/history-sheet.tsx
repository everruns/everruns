"use client";

// History sheet: every recorded change to one entity, newest first, with the
// diff of any point against the current state and "Restore this point".
//
// Decisions:
// - Secrets show only as changed or unchanged. Snapshots carry `$secrets`
//   markers (`{set, fingerprint}`), never values, and the sheet does not
//   surface the fingerprint either: "changed" is all a reader needs.
// - A restore always carries a reason (people may skip one elsewhere, not
//   here: undoing a change is the moment someone later asks "why").
// - A restore never brings secrets back; it keeps their current values. The
//   confirm dialog lists the secrets that differ, and the server's warnings
//   are shown after the restore.
// See knowledge/execution/change-reasons-and-manager-context.md (UI).

import { useMemo, useState } from "react";
import Link from "next/link";
import { ArrowLeft, KeyRound, RotateCcw } from "lucide-react";
import { ChangeReasonField } from "@/components/entity-actions/change-reason-field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import { Skeleton } from "@/components/ui/skeleton";
import {
  useEntityHistory,
  useEntityRevision,
  useEntityRevisionDiff,
  useRestoreEntityRevision,
} from "@/hooks/use-change-history";
import { useMembers } from "@/hooks/use-members";
import { isSecretField, secretFieldName } from "@/lib/api/change-history";
import type { EntityChange, FieldDiff, RestoreResult } from "@/lib/api/types";
import { formatDate, formatRelativeTime } from "@/lib/formatting";
import { entityKindLabel, type EntityKind } from "@/components/entity-actions/entity-kinds";

interface HistorySheetProps {
  entityRef: string;
  kind: EntityKind;
  entityName: string;
  /** Managers may restore; readers only look. */
  canRestore: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

const ACTION_LABELS: Record<string, string> = {
  created: "Created",
  updated: "Updated",
  deleted: "Deleted",
  archived: "Archived",
  unarchived: "Unarchived",
  restored: "Restored",
};

/** "Updated", or "Restored revision 3" for a restore. */
export function describeChangeAction(change: EntityChange): string {
  if (change.action === "restored" || change.restored_from_revision != null) {
    return change.restored_from_revision != null
      ? `Restored revision ${change.restored_from_revision}`
      : "Restored";
  }
  const label = ACTION_LABELS[change.action];
  if (label) return label;
  const words = change.action.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

const SURFACE_LABELS: Record<string, string> = {
  api: "UI or API",
  commands: "CLI",
  mcp: "MCP",
  platform: "Platform Chat",
  worker: "Agent runtime",
  internal: "Internal",
};

export function HistorySheet({
  entityRef,
  kind,
  entityName,
  canRestore,
  open,
  onOpenChange,
}: HistorySheetProps) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const history = useEntityHistory(entityRef, open);
  const entries = useMemo(
    () => history.data?.pages.flatMap((page) => page.entries) ?? [],
    [history.data],
  );
  const selected = entries.find((entry) => entry.id === selectedId) ?? null;

  return (
    <Drawer
      open={open}
      onOpenChange={(next) => {
        if (!next) setSelectedId(null);
        onOpenChange(next);
      }}
    >
      <DrawerContent className="gap-0 overflow-y-auto p-0 sm:max-w-3xl">
        <DrawerHeader className="border-b p-5 pr-12">
          <DrawerTitle>History</DrawerTitle>
          <DrawerDescription>
            Changes to {entityName}, newest first. Open one to compare it with the current{" "}
            {entityKindLabel(kind)}.
          </DrawerDescription>
        </DrawerHeader>
        <div className="flex-1 p-5">
          {selected ? (
            <HistoryEntryDetail
              entityRef={entityRef}
              change={selected}
              canRestore={canRestore}
              onBack={() => setSelectedId(null)}
            />
          ) : history.isLoading ? (
            <div className="space-y-3" aria-label="Loading history">
              <Skeleton className="h-14 w-full" />
              <Skeleton className="h-14 w-full" />
              <Skeleton className="h-14 w-full" />
            </div>
          ) : history.error ? (
            <p role="alert" className="text-sm text-destructive">
              Could not load history: {history.error.message}
            </p>
          ) : entries.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              No changes recorded yet.
            </p>
          ) : (
            <div className="space-y-4">
              <ol className="divide-y border" aria-label="Changes">
                {entries.map((entry) => (
                  <HistoryEntryRow key={entry.id} change={entry} onOpen={setSelectedId} />
                ))}
              </ol>
              {history.hasNextPage && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => history.fetchNextPage()}
                  disabled={history.isFetchingNextPage}
                >
                  {history.isFetchingNextPage ? "Loading..." : "Load older changes"}
                </Button>
              )}
            </div>
          )}
        </div>
      </DrawerContent>
    </Drawer>
  );
}

function useActorName() {
  // Member names are a nicety: a viewer who cannot list members still sees
  // every entry, just without names.
  const { data: members } = useMembers();
  return useMemo(() => {
    const names = new Map((members ?? []).map((m) => [m.user_id, m.name || m.email]));
    return (change: EntityChange): string => {
      const user = change.actor_user_id ? names.get(change.actor_user_id) : undefined;
      switch (change.actor_kind) {
        case "agent_session":
          return user ? `An agent for ${user}` : "An agent";
        case "api_key":
          return user ? `API key of ${user}` : "An API key";
        case "system":
          return "System";
        default:
          return user ?? "A member";
      }
    };
  }, [members]);
}

function HistoryEntryMeta({ change }: { change: EntityChange }) {
  const actorName = useActorName();
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
      <span>{actorName(change)}</span>
      {change.via_session_id && (
        <Link
          href={`/sessions/${change.via_session_id}`}
          className="underline underline-offset-2 hover:text-foreground"
        >
          via {change.via_agent_id ?? "agent"} session
        </Link>
      )}
      <span aria-hidden>·</span>
      <span>{SURFACE_LABELS[change.surface] ?? change.surface}</span>
    </div>
  );
}

function HistoryEntryRow({
  change,
  onOpen,
}: {
  change: EntityChange;
  onOpen: (id: string) => void;
}) {
  return (
    <li className="space-y-1 p-3">
      <div className="flex min-w-0 items-start justify-between gap-3">
        <button
          type="button"
          onClick={() => onOpen(change.id)}
          className="min-w-0 text-left text-sm font-medium hover:underline focus-visible:underline focus-visible:outline-none"
        >
          {describeChangeAction(change)}
          {change.revision != null && (
            <span className="ml-2 font-mono text-xs font-normal text-muted-foreground">
              r{change.revision}
            </span>
          )}
        </button>
        <time
          dateTime={change.created_at}
          title={formatDate(change.created_at)}
          className="shrink-0 text-xs text-muted-foreground"
        >
          {formatRelativeTime(change.created_at)}
        </time>
      </div>
      <HistoryEntryMeta change={change} />
      {change.reason && <p className="text-sm break-words">“{change.reason}”</p>}
    </li>
  );
}

function HistoryEntryDetail({
  entityRef,
  change,
  canRestore,
  onBack,
}: {
  entityRef: string;
  change: EntityChange;
  canRestore: boolean;
  onBack: () => void;
}) {
  const revisionNumber = change.revision ?? null;
  const revision = useEntityRevision(entityRef, revisionNumber);
  const snapshotGone = revision.data !== undefined && revision.data.snapshot == null;
  const diff = useEntityRevisionDiff(entityRef, snapshotGone ? null : revisionNumber);
  const [confirming, setConfirming] = useState(false);
  const [restored, setRestored] = useState<RestoreResult | null>(null);

  const diffs = diff.data ?? [];
  const fieldDiffs = diffs.filter((d) => !isSecretField(d.field));
  const changedSecrets = secretNames(diffs);
  const allSecrets = snapshotSecretNames(revision.data?.snapshot);
  const unchangedSecrets = allSecrets.filter((name) => !changedSecrets.includes(name));

  return (
    <div className="space-y-5">
      <Button variant="ghost" size="sm" onClick={onBack} className="-ml-2">
        <ArrowLeft className="size-4" />
        All changes
      </Button>

      <div className="space-y-1">
        <div className="flex flex-wrap items-baseline gap-2">
          <h3 className="text-base font-semibold">{describeChangeAction(change)}</h3>
          {change.revision != null && (
            <span className="font-mono text-xs text-muted-foreground">r{change.revision}</span>
          )}
          <time dateTime={change.created_at} className="text-xs text-muted-foreground">
            {formatDate(change.created_at)}
          </time>
        </div>
        <HistoryEntryMeta change={change} />
        {change.reason && <p className="text-sm break-words">“{change.reason}”</p>}
        {change.changed_fields.length > 0 && (
          <p className="text-xs text-muted-foreground">
            Changed: <span className="font-mono">{change.changed_fields.join(", ")}</span>
          </p>
        )}
      </div>

      {restored && (
        <Notice variant="success">
          <NoticeTitle>Restored revision {restored.restored_revision}</NoticeTitle>
          {restored.warnings.length > 0 && (
            <NoticeDescription>
              <ul className="list-disc pl-4">
                {restored.warnings.map((warning) => (
                  <li key={warning}>{warning}</li>
                ))}
              </ul>
            </NoticeDescription>
          )}
        </Notice>
      )}

      {revisionNumber === null ? (
        <p className="text-sm text-muted-foreground">
          This change left no snapshot to compare or restore.
        </p>
      ) : revision.isLoading || diff.isLoading ? (
        <Skeleton className="h-32 w-full" />
      ) : snapshotGone ? (
        <p className="text-sm text-muted-foreground">
          This point is older than the snapshots kept, so it can no longer be compared or restored.
        </p>
      ) : revision.error || diff.error ? (
        <p role="alert" className="text-sm text-destructive">
          Could not load this point: {(revision.error ?? diff.error)?.message}
        </p>
      ) : (
        <>
          <section className="space-y-2" aria-label="Difference from the current state">
            <h4 className="text-sm font-medium">Compared with now</h4>
            {diffs.length === 0 ? (
              <p className="text-sm text-muted-foreground">Matches the current state.</p>
            ) : (
              fieldDiffs.length > 0 && <FieldDiffTable diffs={fieldDiffs} />
            )}
            {(changedSecrets.length > 0 || unchangedSecrets.length > 0) && (
              <ul className="space-y-1 text-sm" aria-label="Secrets">
                {changedSecrets.map((name) => (
                  <SecretRow key={name} name={name} changed />
                ))}
                {unchangedSecrets.map((name) => (
                  <SecretRow key={name} name={name} changed={false} />
                ))}
              </ul>
            )}
          </section>

          {canRestore && diffs.length > 0 && (
            <Button variant="outline" onClick={() => setConfirming(true)}>
              <RotateCcw className="size-4" />
              Restore this point
            </Button>
          )}
        </>
      )}

      {revisionNumber !== null && (
        <RestoreDialog
          entityRef={entityRef}
          revision={revisionNumber}
          secretsNotRestored={changedSecrets}
          open={confirming}
          onOpenChange={setConfirming}
          onRestored={(result) => {
            setConfirming(false);
            setRestored(result);
          }}
        />
      )}
    </div>
  );
}

function SecretRow({ name, changed }: { name: string; changed: boolean }) {
  return (
    <li className="flex items-center gap-2">
      <KeyRound className="size-3.5 text-muted-foreground" />
      <span className="font-mono text-xs">{name}</span>
      {changed ? (
        <Badge variant="accent">changed</Badge>
      ) : (
        <Badge variant="outline">unchanged</Badge>
      )}
    </li>
  );
}

/** Names of secrets that differ, from `$secrets.<name>` fields (or a whole `$secrets` field). */
export function secretNames(diffs: FieldDiff[]): string[] {
  const names = new Set<string>();
  for (const diff of diffs) {
    if (!isSecretField(diff.field)) continue;
    if (diff.field === "$secrets") {
      const from = asRecord(diff.from);
      const to = asRecord(diff.to);
      for (const key of new Set([...Object.keys(from), ...Object.keys(to)])) {
        if (JSON.stringify(from[key]) !== JSON.stringify(to[key])) names.add(key);
      }
    } else {
      names.add(secretFieldName(diff.field));
    }
  }
  return [...names].sort();
}

function snapshotSecretNames(snapshot: unknown): string[] {
  return Object.keys(asRecord(asRecord(snapshot)["$secrets"])).sort();
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function renderValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
}

function FieldDiffTable({ diffs }: { diffs: FieldDiff[] }) {
  return (
    <div className="divide-y border text-sm">
      {diffs.map((diff) => (
        <div key={diff.field} className="space-y-1.5 p-3">
          <p className="font-mono text-xs font-medium">{diff.field}</p>
          <div className="grid gap-2 sm:grid-cols-2">
            <div className="min-w-0">
              <p className="text-[11px] text-muted-foreground">At this point</p>
              <pre className="max-h-48 overflow-auto bg-muted/50 p-2 text-xs whitespace-pre-wrap break-words">
                {renderValue(diff.from)}
              </pre>
            </div>
            <div className="min-w-0">
              <p className="text-[11px] text-muted-foreground">Now</p>
              <pre className="max-h-48 overflow-auto bg-muted/50 p-2 text-xs whitespace-pre-wrap break-words">
                {renderValue(diff.to)}
              </pre>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function RestoreDialog({
  entityRef,
  revision,
  secretsNotRestored,
  open,
  onOpenChange,
  onRestored,
}: {
  entityRef: string;
  revision: number;
  secretsNotRestored: string[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRestored: (result: RestoreResult) => void;
}) {
  const [reason, setReason] = useState("");
  const restore = useRestoreEntityRevision(entityRef);
  const canSubmit = reason.trim().length > 0 && !restore.isPending;

  const submit = async () => {
    if (!canSubmit) return;
    try {
      const result = await restore.mutateAsync({ revision, reason });
      setReason("");
      onRestored(result);
    } catch {
      // Shown below from restore.error.
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) restore.reset();
        onOpenChange(next);
      }}
    >
      <DialogContent>
        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>Restore revision {revision}?</DialogTitle>
            <DialogDescription>
              Brings this point back as a new change. The current state stays in history, so this
              can be undone the same way.
            </DialogDescription>
          </DialogHeader>
          {secretsNotRestored.length > 0 && (
            <Notice variant="warning">
              <NoticeTitle>Secrets keep their current values</NoticeTitle>
              <NoticeDescription>
                A restore never brings secrets back. These differ from this point and will not be
                restored: <span className="font-mono">{secretsNotRestored.join(", ")}</span>
              </NoticeDescription>
            </Notice>
          )}
          <ChangeReasonField value={reason} onChange={setReason} required />
          {restore.error && (
            <p role="alert" className="text-sm text-destructive">
              Could not restore: {restore.error.message}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {restore.isPending ? "Restoring..." : "Restore"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
