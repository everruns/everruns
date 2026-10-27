"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import type { ReactNode } from "react";
import { Check, Loader2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { describeCronExpression } from "@/components/apps/cron-label";
import { useAgentNameAvailability } from "@/hooks/use-name-availability";
import { useModels } from "@/hooks/use-providers";
import type { AgentDraft } from "@/lib/api/agent-draft";
import type { Capability } from "@/lib/api/types";
import { DRAFT_CHANNEL_LABELS, draftBlocker } from "@/lib/new-agent";

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="border-t border-border/60 px-4 py-3">
      <div className="mb-1 text-xs font-medium text-muted-foreground">{label}</div>
      <div className="text-sm">{children}</div>
    </div>
  );
}

function NotSet() {
  const ui = useUiTranslations();
  return <span className="text-muted-foreground italic">{ui("Not set yet")}</span>;
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
  const ui = useUiTranslations();
  const availability = useAgentNameAvailability(draft.name);
  const blocker =
    draftBlocker(draft) ??
    (availability.available === false
      ? `An agent named ${draft.name} already exists. Change it with Edit as form`
      : null);
  const title = draft.display_name.trim() || "Untitled agent";
  const capabilityName = (id: string) => capabilities?.find((c) => c.id === id)?.name ?? id;
  const { data: models } = useModels();
  const pinnedModel = draft.model_id
    ? (models?.find((m) => m.id === draft.model_id)?.display_name ?? draft.model_id)
    : null;
  const github = draft.github_trigger;

  return (
    <aside className="rounded-md border bg-card" aria-label={ui("Agent draft")}>
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
        <Badge variant="secondary">{ui("Draft")}</Badge>
      </div>
      <Field label={ui("Instructions")}>
        {draft.system_prompt.trim() ? (
          <p className="line-clamp-6 whitespace-pre-wrap">{draft.system_prompt}</p>
        ) : (
          <NotSet />
        )}
      </Field>
      <Field label={ui("Wakes itself")}>
        {draft.schedule ? (
          <span>
            {describeCronExpression(draft.schedule.cron)} ({draft.schedule.timezone})
          </span>
        ) : (
          <NotSet />
        )}
      </Field>
      {github && (
        <Field label={ui("Starts on GitHub")}>
          <span>
            {github.events.join(", ")}
            {github.repositories.length > 0 && ` in ${github.repositories.join(", ")}`}
            {(github.workflows?.length ?? 0) > 0 && `, workflow ${github.workflows?.join(", ")}`}
            {(github.branches?.length ?? 0) > 0 && ` on ${github.branches?.join(", ")}`}
            {github.changes_only && " when the outcome changes"}
            <span className="block text-xs text-muted-foreground">
              {ui("You connect GitHub to the agent after it is created.")}
            </span>
          </span>
        </Field>
      )}
      <Field label={ui("Uses")}>
        {draft.capabilities.length > 0 ? (
          draft.capabilities.map(capabilityName).join(", ")
        ) : (
          <NotSet />
        )}
      </Field>
      <Field label={ui("Ways in")}>
        {draft.channels.length > 0 ? (
          <span>
            {draft.channels.map((kind) => DRAFT_CHANNEL_LABELS[kind].label).join(", ")}
            <span className="block text-xs text-muted-foreground">
              {ui("Created as drafts. Nothing takes traffic until you publish.")}
            </span>
          </span>
        ) : (
          <NotSet />
        )}
      </Field>
      {(draft.mcp_servers?.length ?? 0) > 0 && (
        <Field label={ui("Connects to")}>
          {draft.mcp_servers?.map((server) => server.name).join(", ")}
        </Field>
      )}
      <Field label={ui("Model and harness")}>
        <span className="text-muted-foreground">
          {pinnedModel
            ? ui("{value0}, {value1} default harness", { value0: pinnedModel, value1: orgName })
            : ui("{value0} defaults", { value0: orgName })}
        </span>
      </Field>
      {blocker && draft.display_name.trim() && (
        <p className="border-t border-border/60 px-4 py-3 text-xs text-muted-foreground">
          {blocker}.
        </p>
      )}
      <div className="flex flex-wrap items-center justify-end gap-2 border-t border-border/60 px-4 py-3">
        <Button type="button" variant="outline" onClick={onEdit}>
          {ui("Edit as form")}
        </Button>
        <Button
          type="button"
          variant="accent"
          onClick={onCreate}
          disabled={pending || blocker !== null}
          title={blocker ?? undefined}
        >
          {pending ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
          {ui("Create agent")}
        </Button>
      </div>
    </aside>
  );
}
