"use client";

import type { ReactNode } from "react";
import { Check, Loader2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { describeCronExpression } from "@/components/apps/cron-label";
import { useAgentNameAvailability } from "@/hooks/use-name-availability";
import type { AgentDraft } from "@/lib/api/agent-draft";
import type { Capability } from "@/lib/api/types";
import { DRAFT_CHANNEL_LABELS, draftBlocker } from "@/lib/new-agent";

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="border-t border-border/60 px-4 py-3">
      <div className="mb-1 font-mono text-[11px] tracking-wider text-muted-foreground uppercase">
        {label}
      </div>
      <div className="text-sm">{children}</div>
    </div>
  );
}

function NotSet() {
  return <span className="text-muted-foreground italic">Not set yet</span>;
}

/** The live draft next to the agent builder. Read-only; Blank edits it. */
export function DraftCard({
  draft,
  capabilities,
  orgName,
  pending,
  onCreate,
  onEdit,
}: {
  draft: AgentDraft;
  capabilities?: Capability[];
  orgName: string;
  pending: boolean;
  onCreate: () => void;
  onEdit: () => void;
}) {
  const availability = useAgentNameAvailability(draft.name);
  const blocker =
    draftBlocker(draft) ??
    (availability.available === false
      ? `An agent named ${draft.name} already exists. Change it with Edit as form`
      : null);
  const title = draft.display_name.trim() || "Untitled agent";
  const capabilityName = (id: string) => capabilities?.find((c) => c.id === id)?.name ?? id;

  return (
    <aside className="rounded-md border bg-card" aria-label="Agent draft">
      <div className="flex items-start gap-3 px-4 py-4">
        <div className="flex size-8 shrink-0 items-center justify-center rounded bg-accent/15 text-sm font-semibold">
          {title.charAt(0).toUpperCase()}
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate font-medium">{title}</div>
          <div className="truncate font-mono text-xs text-muted-foreground">
            {draft.name || "–"}
          </div>
        </div>
        <Badge variant="secondary">Draft</Badge>
      </div>
      <Field label="Instructions">
        {draft.system_prompt.trim() ? (
          <p className="line-clamp-6 whitespace-pre-wrap">{draft.system_prompt}</p>
        ) : (
          <NotSet />
        )}
      </Field>
      <Field label="Wakes itself">
        {draft.schedule ? (
          <span>
            {describeCronExpression(draft.schedule.cron)} ({draft.schedule.timezone})
          </span>
        ) : (
          <NotSet />
        )}
      </Field>
      <Field label="Uses">
        {draft.capabilities.length > 0 ? (
          draft.capabilities.map(capabilityName).join(", ")
        ) : (
          <NotSet />
        )}
      </Field>
      <Field label="Ways in">
        {draft.channels.length > 0 ? (
          <span>
            {draft.channels.map((kind) => DRAFT_CHANNEL_LABELS[kind].label).join(", ")}
            <span className="block text-xs text-muted-foreground">
              Created as drafts. Nothing takes traffic until you publish.
            </span>
          </span>
        ) : (
          <NotSet />
        )}
      </Field>
      <Field label="Model and harness">
        <span className="text-muted-foreground">{orgName} defaults</span>
      </Field>
      {blocker && draft.display_name.trim() && (
        <p className="border-t border-border/60 px-4 py-3 text-xs text-muted-foreground">
          {blocker}.
        </p>
      )}
      <div className="flex flex-wrap items-center justify-end gap-2 border-t border-border/60 px-4 py-3">
        <Button type="button" variant="outline" onClick={onEdit}>
          Edit as form
        </Button>
        <Button
          type="button"
          variant="accent"
          onClick={onCreate}
          disabled={pending || blocker !== null}
          title={blocker ?? undefined}
        >
          {pending ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
          Create agent
        </Button>
      </div>
    </aside>
  );
}
