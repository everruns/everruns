"use client";

// One Sandbox: what it is, where it runs, what happened to it, and the actions
// its Session allows. Actions go through the Session's Sandbox endpoint, so a
// Sandbox whose Session is gone is read-only.

import { useState } from "react";
import { AlertTriangle, Pause, Play, Trash2 } from "lucide-react";
import { useManageSandbox, useSandbox } from "@/hooks/use-sandbox-fleet";
import type {
  SandboxAction,
  SandboxFleetDetail,
  SandboxStateSpan,
} from "@/lib/api/sandboxes";
import { Button, LinkButton } from "@/components/ui/button";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { Skeleton } from "@/components/ui/skeleton";
import { formatDate } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import {
  SANDBOX_ATTENTION_COPY,
  SandboxStateBadge,
  formatRunningTime,
  providerLabel,
  sandboxStateLabel,
  sandboxStateTone,
} from "./sandbox-display";
import { sandboxTitle } from "./sandbox-fleet-table";
import type { SandboxAttention } from "@/lib/api/sandboxes";

const TARGET_LABELS: Record<string, string> = {
  managed: "Managed sandbox",
  container: "Container",
  machine: "Registered machine",
  vfs: "Virtual filesystem",
  host: "This machine",
};

// What the Sandbox is being driven toward, in the same words as its state.
const DESIRED_LABELS: Record<string, string> = {
  ready: "Running",
  paused: "Paused",
  deleted: "Deleted",
};

function spanSeconds(span: SandboxStateSpan): number {
  return Math.max(
    0,
    Math.round(
      (new Date(span.end).getTime() - new Date(span.start).getTime()) / 1000,
    ),
  );
}

/** Newest first, with the rebuild between two generations called out. */
function History({ history }: { history: SandboxStateSpan[] }) {
  const rows = [...history].reverse();
  return (
    <ol className="flex flex-col border-l border-border pl-4">
      {rows.map((span, index) => {
        const previous = rows[index + 1];
        const rebuilt = previous && span.generation > previous.generation;
        return (
          <li key={`${span.start}-${index}`} className="relative pb-3 text-sm">
            <span
              aria-hidden="true"
              className={cn(
                "absolute top-1.5 -left-[21px] size-2.5 rounded-full border-2 border-background",
                sandboxStateTone(span.state),
              )}
            />
            <span className="font-medium">
              {rebuilt ? `Rebuilt as generation ${span.generation}, ` : ""}
              {sandboxStateLabel(span.state).toLowerCase()}
            </span>
            <span className="block font-mono text-[11px] text-muted-foreground">
              {formatDate(span.start)}
              {span.current
                ? ", now"
                : `, for ${formatRunningTime(spanSeconds(span))}`}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

function Facts({ detail }: { detail: SandboxFleetDetail }) {
  const rows: Array<[string, React.ReactNode]> = [
    ["State", <SandboxStateBadge key="s" state={detail.state} />],
    ["Wants", DESIRED_LABELS[detail.desired_state] ?? detail.desired_state],
    ["Provider", providerLabel(detail.provider)],
    [
      "Target",
      TARGET_LABELS[detail.target_kind ?? ""] ??
        detail.target_kind ??
        "Unknown",
    ],
    [
      "Template",
      detail.template
        ? `${detail.template.display_name ?? "Template"}, revision ${detail.template.revision ?? "?"}`
        : "No template",
    ],
    [
      "Provider id",
      <span key="e" className="font-mono text-xs">
        {detail.external_id ?? "None yet"}
      </span>,
    ],
    [
      "Workspace",
      <span key="w" className="font-mono text-xs">
        {detail.workspace_path ?? "Unknown"}
      </span>,
    ],
    [
      "Idle pause",
      detail.idle_after_seconds
        ? `after ${formatRunningTime(detail.idle_after_seconds)} idle`
        : "Off",
    ],
    ["Checkpoints", String(detail.checkpoint_count)],
    ["Role", detail.role === "primary" ? "Primary" : "Resource"],
    ["Created", formatDate(detail.created_at)],
  ];
  if (detail.deleted_at) rows.push(["Deleted", formatDate(detail.deleted_at)]);
  return (
    <dl className="grid grid-cols-[7.5rem_1fr] gap-x-3 gap-y-1.5 text-sm">
      {rows.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-muted-foreground">{label}</dt>
          <dd className="min-w-0 break-words">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function Actions({ detail }: { detail: SandboxFleetDetail }) {
  const manage = useManageSandbox();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const sessionId = detail.session_id;
  if (!sessionId || detail.state === "deleted") return null;
  const run = (action: SandboxAction) => manage.mutate({ sessionId, action });
  const canPause = detail.state === "running";
  // A lost Sandbox is rebuilt by its next tool call, not by an action here.
  const canResume = detail.state === "paused";
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap gap-2">
        {canPause ? (
          <Button
            size="sm"
            variant="outline"
            onClick={() => run("pause")}
            disabled={manage.isPending}
          >
            <Pause className="size-4" /> Pause
          </Button>
        ) : null}
        {canResume ? (
          <Button
            size="sm"
            variant="outline"
            onClick={() => run("resume")}
            disabled={manage.isPending}
          >
            <Play className="size-4" /> Resume
          </Button>
        ) : null}
        {confirmDelete ? (
          <>
            <Button
              size="sm"
              variant="destructive"
              onClick={() => run("delete")}
              disabled={manage.isPending}
            >
              Delete Sandbox and workspace
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setConfirmDelete(false)}
            >
              Keep it
            </Button>
          </>
        ) : (
          <Button
            size="sm"
            variant="outline"
            onClick={() => setConfirmDelete(true)}
          >
            <Trash2 className="size-4" /> Delete
          </Button>
        )}
      </div>
      {manage.error ? (
        <p className="text-sm text-destructive">
          {manage.error instanceof Error
            ? manage.error.message
            : "The action failed."}
        </p>
      ) : null}
    </div>
  );
}

export function SandboxDetailDrawer({
  sandboxId,
  onClose,
}: {
  sandboxId: string | null;
  onClose: () => void;
}) {
  const { data: detail, isLoading, error } = useSandbox(sandboxId);
  return (
    <Drawer
      open={sandboxId !== null}
      onOpenChange={(open) => !open && onClose()}
    >
      <DrawerContent className="gap-0 overflow-y-auto p-0 sm:max-w-lg">
        <DrawerHeader className="border-b p-5 pr-12">
          <DrawerTitle>{detail ? sandboxTitle(detail) : "Sandbox"}</DrawerTitle>
          <DrawerDescription className="font-mono text-xs">
            {sandboxId}
          </DrawerDescription>
        </DrawerHeader>
        <div className="flex flex-col gap-6 p-5">
          {isLoading ? (
            <div className="flex flex-col gap-2">
              <Skeleton className="h-4 w-2/3" />
              <Skeleton className="h-4 w-1/2" />
              <Skeleton className="h-4 w-3/4" />
            </div>
          ) : error || !detail ? (
            <p className="text-sm text-destructive">
              This Sandbox could not be loaded.
            </p>
          ) : (
            <>
              {detail.attention.length > 0 ? (
                <div className="flex flex-col gap-2 border border-destructive/30 bg-destructive/5 p-3">
                  {detail.attention.map((reason) => {
                    const copy =
                      SANDBOX_ATTENTION_COPY[reason as SandboxAttention];
                    return (
                      <div key={reason} className="flex gap-2 text-sm">
                        <AlertTriangle className="mt-0.5 size-4 shrink-0 text-destructive" />
                        <div>
                          <p className="font-medium">{copy?.title ?? reason}</p>
                          {copy ? (
                            <p className="text-muted-foreground">{copy.hint}</p>
                          ) : null}
                          {reason === "init_failed" &&
                          detail.last_init_error ? (
                            <pre className="mt-1 overflow-x-auto bg-muted p-2 font-mono text-xs">
                              {detail.last_init_error}
                            </pre>
                          ) : null}
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : null}
              <Facts detail={detail} />
              <div className="flex flex-wrap gap-2">
                {detail.session_id ? (
                  <LinkButton size="sm" href={`/sessions/${detail.session_id}`}>
                    Open session
                  </LinkButton>
                ) : null}
                {detail.agent_id ? (
                  <LinkButton
                    size="sm"
                    variant="outline"
                    href={`/agents/${detail.agent_id}`}
                  >
                    {detail.agent_name ?? "Agent"}
                  </LinkButton>
                ) : null}
              </div>
              <Actions detail={detail} />
              <section className="flex flex-col gap-3">
                <h3 className="font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
                  History
                </h3>
                {detail.history.length > 0 ? (
                  <History history={detail.history} />
                ) : (
                  <p className="text-sm text-muted-foreground">
                    No state changes recorded yet.
                  </p>
                )}
              </section>
              {detail.incarnations.length > 0 ? (
                <section className="flex flex-col gap-2">
                  <h3 className="font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
                    Provider resources
                  </h3>
                  <ul className="flex flex-col divide-y border text-sm">
                    {detail.incarnations.map((incarnation) => (
                      <li
                        key={incarnation.generation}
                        className="flex flex-col gap-0.5 p-2.5"
                      >
                        <span className="flex items-center justify-between gap-2">
                          <span className="font-mono text-xs">
                            {incarnation.external_id}
                          </span>
                          <span className="text-xs text-muted-foreground">
                            Generation {incarnation.generation}
                          </span>
                        </span>
                        <span className="text-xs text-muted-foreground">
                          {formatDate(incarnation.created_at)}
                          {incarnation.retired_at
                            ? ` to ${formatDate(incarnation.retired_at)}`
                            : ", current"}
                        </span>
                      </li>
                    ))}
                  </ul>
                </section>
              ) : null}
              {!detail.session_id ? (
                <p className="text-xs text-muted-foreground">
                  Its Session was deleted. The record stays here as history for
                  a limited time.
                </p>
              ) : null}
            </>
          )}
        </div>
      </DrawerContent>
    </Drawer>
  );
}
