"use client";

// The harness page's narrow config column. Primary settings (parent harness,
// model, tags) are always visible; everything set once and rarely changed,
// including the optional system prompt, is one "More" row each that shows its
// current value and opens a side sheet. Editable values wear a bordered
// control; read-only facts (Updated, and every value on an archived or
// built-in harness) are plain muted text.
//
// The controls are live in view mode too: changing one puts the page into
// edit mode with that change pending, so nothing is saved until Save.

import Link from "next/link";
import { Badge } from "@/components/ui/badge";
import { TagInput } from "@/components/ui/tag-input";
import { HarnessSelect } from "@/components/harness/harness-select";
import { ModelPicker } from "@/components/models/model-picker";
import type { HarnessDraft } from "@/components/harnesses/use-harness-draft";
import type { Harness, ModelWithProvider } from "@/lib/api/types";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { normalizeTags } from "@/lib/tags";
import { MoreSettingsNav, type MoreSettingsRow } from "@/components/workspace/more-settings-nav";
import { cn } from "@/lib/utils";

export type HarnessMoreRow = MoreSettingsRow;

interface HarnessConfigColumnProps {
  harness: Harness;
  parentHarness?: Harness;
  draft: HarnessDraft;
  editing: boolean;
  readOnly: boolean;
  defaultModel?: ModelWithProvider;
  onStartEdit: () => void;
  moreRows: HarnessMoreRow[];
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

export function HarnessConfigColumn({
  harness,
  parentHarness,
  draft,
  editing,
  readOnly,
  defaultModel,
  onStartEdit,
  moreRows,
  onOpenRow,
  className,
}: HarnessConfigColumnProps) {
  // Any control change starts (or continues) the page-level edit.
  const edit = <T,>(apply: (value: T) => void) => {
    return (value: T) => {
      apply(value);
      if (!editing) onStartEdit();
    };
  };

  return (
    <aside
      aria-label="Harness configuration"
      className={cn("flex min-w-0 flex-col bg-muted/30", className)}
    >
      <div className="flex flex-col gap-4 border-b p-4">
        <div className="flex flex-col gap-1.5">
          <FieldLabel htmlFor="parent-harness">Parent harness</FieldLabel>
          {readOnly ? (
            parentHarness ? (
              <Link
                href={`/harnesses/${parentHarness.id}`}
                className="text-[13px] text-primary hover:underline"
              >
                {getDisplayName(parentHarness)}
              </Link>
            ) : (
              <span className="text-[13px] text-muted-foreground">
                {harness.parent_harness_id || "None"}
              </span>
            )
          ) : (
            <HarnessSelect
              id="parent-harness"
              value={draft.fields.parent_harness_id}
              onValueChange={edit((value: string) => draft.setField("parent_harness_id", value))}
              placeholder="No parent harness"
              includeNoneOption
              noneLabel="No parent harness"
              excludeIds={[harness.id]}
              className="w-full bg-background"
            />
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
              {normalizeTags(harness.tags).length === 0 ? (
                <span className="text-[13px] text-muted-foreground">No tags</span>
              ) : (
                normalizeTags(harness.tags).map((tag) => (
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
            {new Date(harness.updated_at).toLocaleString()}
          </span>
        </div>
      </div>

      <MoreSettingsNav rows={moreRows} onOpen={onOpenRow} />
    </aside>
  );
}
