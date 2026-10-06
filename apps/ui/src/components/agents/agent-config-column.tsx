"use client";

// The agent page's narrow config column. Primary settings (harness,
// capabilities, model, tags) are always visible; everything set once and
// rarely changed is one "More" row each that shows its current value and
// opens a side sheet. Editable values wear a bordered control; read-only facts
// (Updated, and every value on an archived agent) are plain muted text.
//
// The controls are live in view mode too: changing one puts the page into
// edit mode with that change pending, so nothing is saved until Save.

import { ChevronRight, Pencil } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { TagInput } from "@/components/ui/tag-input";
import { HarnessSelect } from "@/components/harness/harness-select";
import { ModelPicker } from "@/components/models/model-picker";
import { CapabilitySelector } from "@/components/agents/capability-selector";
import type { AgentDraft } from "@/components/agents/use-agent-draft";
import type { Agent, Capability, ModelWithProvider } from "@/lib/api/types";
import { AgentCapabilityList } from "./agent-capability-list";
import { normalizeTags } from "@/lib/tags";
import { cn } from "@/lib/utils";

export interface AgentMoreRow {
  id: string;
  label: string;
  summary: string;
}

interface AgentConfigColumnProps {
  agent: Agent;
  draft: AgentDraft;
  editing: boolean;
  readOnly: boolean;
  allCapabilities: Capability[];
  defaultModel?: ModelWithProvider;
  onStartEdit: () => void;
  moreRows: AgentMoreRow[];
  onOpenRow: (id: string) => void;
  className?: string;
}

function FieldLabel({ htmlFor, children }: { htmlFor?: string; children: React.ReactNode }) {
  return (
    <label htmlFor={htmlFor} className="text-xs font-medium text-muted-foreground">
      {children}
    </label>
  );
}

export function AgentConfigColumn({
  agent,
  draft,
  editing,
  readOnly,
  allCapabilities,
  defaultModel,
  onStartEdit,
  moreRows,
  onOpenRow,
  className,
}: AgentConfigColumnProps) {
  // Any control change starts (or continues) the page-level edit.
  const edit = <T,>(apply: (value: T) => void) => {
    return (value: T) => {
      apply(value);
      if (!editing) onStartEdit();
    };
  };

  return (
    <aside
      aria-label="Agent configuration"
      className={cn("flex min-w-0 flex-col bg-muted/30", className)}
    >
      <div className="flex flex-col gap-4 border-b p-4">
        <div className="flex flex-col gap-1.5">
          <FieldLabel htmlFor="harness">Harness</FieldLabel>
          {readOnly ? (
            <span className="text-[13px] text-muted-foreground">
              {agent.effective_harness?.display_name || agent.effective_harness?.name || "Default"}
            </span>
          ) : (
            <HarnessSelect
              id="harness"
              value={draft.fields.harness_id}
              onValueChange={edit((value: string) => draft.setField("harness_id", value))}
              placeholder="Select a harness"
              className="w-full bg-background"
            />
          )}
          {draft.errors.harness_id && (
            <p className="text-xs text-destructive">{draft.errors.harness_id}</p>
          )}
        </div>

        <div className="flex flex-col gap-1.5">
          {editing ? (
            <CapabilitySelector
              capabilities={allCapabilities}
              selected={draft.capabilities}
              onChange={draft.setCapabilities}
              label="Capabilities"
            />
          ) : (
            <>
              <div className="flex items-center gap-2">
                <span className="flex-1 text-xs font-medium text-muted-foreground">
                  Capabilities
                </span>
                {!readOnly && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-6 px-1.5 text-xs"
                    onClick={onStartEdit}
                    aria-label="Edit capabilities"
                  >
                    <Pencil className="size-3" />
                    Edit
                  </Button>
                )}
              </div>
              <AgentCapabilityList
                references={draft.capabilities.map((config) => config.ref)}
                capabilities={allCapabilities}
              />
            </>
          )}
        </div>

        <div className="flex flex-col gap-1.5">
          <FieldLabel htmlFor="default-model">Default model</FieldLabel>
          {readOnly ? (
            <span className="text-[13px] text-muted-foreground">
              {defaultModel?.display_name ?? "Organization default"}
            </span>
          ) : (
            <ModelPicker
              id="default-model"
              value={draft.fields.default_model_id}
              onChange={edit((value: string) => draft.setField("default_model_id", value))}
              placeholder="Use default model"
              className="w-full bg-background"
            />
          )}
        </div>

        <div className="flex flex-col gap-1.5">
          <FieldLabel htmlFor="tags">Tags</FieldLabel>
          {readOnly ? (
            <div className="flex flex-wrap gap-1">
              {normalizeTags(agent.tags).length === 0 ? (
                <span className="text-[13px] text-muted-foreground">No tags</span>
              ) : (
                normalizeTags(agent.tags).map((tag) => (
                  <Badge key={tag} variant="outline">
                    {tag}
                  </Badge>
                ))
              )}
            </div>
          ) : (
            <TagInput
              id="tags"
              placeholder="Add a tag…"
              value={draft.fields.tags}
              onChange={edit((value: string) => draft.setField("tags", value))}
              className="bg-background"
            />
          )}
        </div>

        <div className="flex flex-col gap-0.5">
          <span className="text-[13px] font-medium">Updated</span>
          <span className="text-[13px] text-muted-foreground">
            {new Date(agent.updated_at).toLocaleString()}
          </span>
        </div>
      </div>

      <nav aria-label="More settings" className="flex flex-col pt-2 pb-4">
        <p className="px-4 pt-2 pb-1 font-mono text-[11px] tracking-[0.08em] text-muted-foreground uppercase">
          More
        </p>
        {moreRows.map((row) => (
          <button
            key={row.id}
            type="button"
            onClick={() => onOpenRow(row.id)}
            className="flex items-center gap-2 px-4 py-2 text-left text-[13px] transition-colors hover:bg-muted"
          >
            <span className="flex-1">{row.label}</span>
            <span className="min-w-0 truncate text-muted-foreground">{row.summary}</span>
            <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" />
          </button>
        ))}
      </nav>
    </aside>
  );
}
