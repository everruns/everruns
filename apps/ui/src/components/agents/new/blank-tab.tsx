"use client";

import { useState } from "react";
import { Check, Loader2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { describeCronExpression } from "@/components/apps/cron-label";
import { useAgentNameAvailability } from "@/hooks/use-name-availability";
import type { AgentDraft, DraftChannelKind } from "@/lib/api/agent-draft";
import {
  DRAFT_CHANNEL_KINDS,
  DRAFT_CHANNEL_LABELS,
  draftBlocker,
  slugifyAgentName,
} from "@/lib/new-agent";
import { cn } from "@/lib/utils";

/** Name, instructions and ways in; everything else takes the org defaults. */
export function BlankTab({
  draft,
  onDraftChange,
  orgName,
  creating,
  onCreate,
}: {
  draft: AgentDraft;
  onDraftChange: (draft: AgentDraft) => void;
  orgName: string;
  creating: boolean;
  onCreate: () => void;
}) {
  // Follow the display name until the person types their own slug.
  const [slugEdited, setSlugEdited] = useState(
    () => draft.name !== "" && draft.name !== slugifyAgentName(draft.display_name),
  );
  const availability = useAgentNameAvailability(draft.name);
  const blocker =
    draftBlocker(draft) ?? (availability.available === false ? "That name is taken" : null);

  const update = (patch: Partial<AgentDraft>) => onDraftChange({ ...draft, ...patch });
  const toggleChannel = (kind: DraftChannelKind) =>
    update({
      channels: draft.channels.includes(kind)
        ? draft.channels.filter((c) => c !== kind)
        : [...draft.channels, kind],
    });

  return (
    <form
      className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_320px]"
      onSubmit={(event) => {
        event.preventDefault();
        if (!blocker) onCreate();
      }}
    >
      <div className="space-y-6">
        <section className="space-y-4 rounded-md border bg-card p-4">
          <h2 className="font-medium">The job</h2>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="blank-display-name">Name</Label>
              <Input
                id="blank-display-name"
                value={draft.display_name}
                placeholder="Invoice chaser"
                onChange={(event) =>
                  update({
                    display_name: event.target.value,
                    ...(slugEdited ? {} : { name: slugifyAgentName(event.target.value) }),
                  })
                }
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="blank-name">Addressable name</Label>
              <div className="relative">
                <Input
                  id="blank-name"
                  value={draft.name}
                  placeholder="invoice-chaser"
                  className="pr-8 font-mono"
                  onChange={(event) => {
                    setSlugEdited(true);
                    update({ name: event.target.value });
                  }}
                />
                {draft.name && !availability.isChecking && availability.available !== null && (
                  <span className="absolute top-1/2 right-2 -translate-y-1/2">
                    {availability.available ? (
                      <Check className="size-4 text-success" aria-label="Name available" />
                    ) : (
                      <X className="size-4 text-destructive" aria-label="Name taken" />
                    )}
                  </span>
                )}
              </div>
            </div>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="blank-description">Description</Label>
            <Input
              id="blank-description"
              value={draft.description}
              placeholder="One sentence on what it does"
              onChange={(event) => update({ description: event.target.value })}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="blank-instructions">Instructions</Label>
            <Textarea
              id="blank-instructions"
              value={draft.system_prompt}
              rows={8}
              placeholder="You chase overdue invoices. Each weekday morning..."
              onChange={(event) => update({ system_prompt: event.target.value })}
            />
          </div>
          {draft.schedule && (
            <div className="flex items-center justify-between gap-3 rounded border px-3 py-2 text-sm">
              <span>
                Wakes itself: {describeCronExpression(draft.schedule.cron)} (
                {draft.schedule.timezone}): “{draft.schedule.message}”
              </span>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => update({ schedule: null })}
              >
                Remove
              </Button>
            </div>
          )}
        </section>

        <section className="space-y-3 rounded-md border bg-card p-4">
          <div>
            <h2 className="font-medium">Where it works</h2>
            <p className="text-sm text-muted-foreground">
              Each one is created as a draft channel. Nothing takes traffic until you publish it.
            </p>
          </div>
          <div className="grid gap-2 sm:grid-cols-2">
            {DRAFT_CHANNEL_KINDS.map((kind) => {
              const selected = draft.channels.includes(kind);
              return (
                <button
                  key={kind}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => toggleChannel(kind)}
                  className={cn(
                    "rounded-md border px-3 py-2 text-left transition-colors",
                    selected ? "border-accent bg-accent/10" : "hover:bg-muted/50",
                  )}
                >
                  <div className="text-sm font-medium">{DRAFT_CHANNEL_LABELS[kind].label}</div>
                  <div className="text-xs text-muted-foreground">
                    {DRAFT_CHANNEL_LABELS[kind].hint}
                  </div>
                </button>
              );
            })}
          </div>
        </section>
      </div>

      <aside className="h-fit space-y-3 rounded-md border bg-card p-4">
        <p className="text-sm text-muted-foreground">
          Model and harness use {orgName} defaults. Change them, or add capabilities, on the agent
          page after it is created.
        </p>
        {blocker && <p className="text-sm text-muted-foreground">{blocker}.</p>}
        <Button type="submit" variant="accent" className="w-full" disabled={creating || !!blocker}>
          {creating ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
          Create agent
        </Button>
      </aside>
    </form>
  );
}
