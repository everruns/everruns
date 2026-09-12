import { Plus, Trash2 } from "lucide-react";

import { HarnessIcon } from "@/lib/harness-icons";
import type { ConversationStarter } from "@/lib/api/legacy-api-types";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export const MAX_STARTERS = 8;

export interface StartersEditorProps {
  /** Current starters (at most 8). */
  value: ConversationStarter[];
  /** Called with the updated list. */
  onChange: (value: ConversationStarter[]) => void;
  /** Validation message for the list, if any. */
  error?: string;
  /** Optional per-row text errors, aligned by index. */
  rowErrors?: (string | undefined)[];
}

/**
 * Editor for Platform Chat conversation starters. Each starter has prompt
 * text plus an optional icon name from the harness icon set (`HarnessIcon`
 * falls back to the default glyph for unknown names).
 */
export function StartersEditor({ value, onChange, error, rowErrors = [] }: StartersEditorProps) {
  const update = (index: number, patch: Partial<ConversationStarter>) => {
    onChange(value.map((starter, i) => (i === index ? { ...starter, ...patch } : starter)));
  };

  const remove = (index: number) => {
    onChange(value.filter((_, i) => i !== index));
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <Label>Conversation starters</Label>
        <span className="text-xs text-muted-foreground">
          {value.length}/{MAX_STARTERS} · shown on a fresh Platform Chat thread
        </span>
      </div>
      {value.length === 0 ? (
        <p className="rounded-lg border border-dashed px-3 py-4 text-center text-sm text-muted-foreground">
          No starters yet. Add up to {MAX_STARTERS} prompts users can insert with one click.
        </p>
      ) : (
        <ul className="space-y-2">
          {value.map((starter, index) => (
            <li key={index} className="flex items-center gap-2">
              <span className="flex size-9 shrink-0 items-center justify-center rounded-lg border bg-muted/50">
                <HarnessIcon
                  icon={starter.icon || undefined}
                  className="size-4 text-muted-foreground"
                />
              </span>
              <Input
                value={starter.icon ?? ""}
                onChange={(event) => update(index, { icon: event.target.value || null })}
                placeholder="Icon (e.g. zap)"
                maxLength={64}
                className="w-32 shrink-0"
                aria-label={`Starter ${index + 1} icon`}
              />
              <Input
                value={starter.text}
                onChange={(event) => update(index, { text: event.target.value })}
                placeholder="Starter prompt, e.g. Triage the newest P1"
                maxLength={280}
                aria-label={`Starter ${index + 1} text`}
              />
              <button
                type="button"
                onClick={() => remove(index)}
                className="flex size-9 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-destructive"
                aria-label={`Remove starter ${index + 1}`}
              >
                <Trash2 className="size-4" />
              </button>
            </li>
          ))}
        </ul>
      )}
      {rowErrors.map((rowError, index) =>
        rowError ? (
          <p key={index} className="text-sm text-destructive">
            Starter {index + 1}: {rowError}
          </p>
        ) : null,
      )}
      {error ? <p className="text-sm text-destructive">{error}</p> : null}
      <button
        type="button"
        onClick={() => onChange([...value, { icon: null, text: "" }])}
        disabled={value.length >= MAX_STARTERS}
        className="inline-flex items-center gap-1.5 rounded-lg border border-dashed px-3 py-2 text-sm text-muted-foreground transition-colors hover:border-solid hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
      >
        <Plus className="size-4" />
        Add starter
      </button>
    </div>
  );
}
