"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import type { FieldDiff } from "@/lib/api/types";
import { isSecretField } from "@/lib/api/change-history";

function valueText(value: unknown): string {
  if (value === undefined) return "Not set";
  if (value === null) return "None";
  return typeof value === "string" ? value || "Empty" : JSON.stringify(value, null, 2);
}

/** Only explicit draft projections or sanitized history diffs belong here. */
export function PageChangeDiff({
  fields,
  pending = false,
  canUndo = true,
  undo,
  fieldLabels,
}: {
  fields: readonly FieldDiff[];
  fieldLabels?: Record<string, string>;
  pending?: boolean;
  canUndo?: boolean;
  undo?: (field: string) => void;
}) {
  const ui = useUiTranslations();
  const visible = fields.filter((field) => !isSecretField(field.field));
  if (!visible.length) return null;
  return (
    <details className="mt-3 min-w-0" open>
      <summary className="cursor-pointer text-sm font-medium">
        {ui("Review changes (")}
        {visible.length})
      </summary>
      <div className="mt-2 divide-y border bg-background">
        {visible.map((diff) => (
          <section
            key={diff.field}
            className="min-w-0 space-y-2 p-3"
            aria-label={ui("Change to {value0}", { value0: diff.field })}
          >
            <div className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium">
                {fieldLabels?.[diff.field] ?? diff.field.replaceAll("_", " ")}
              </span>
              {undo && (
                <button
                  type="button"
                  className="text-xs underline disabled:opacity-50"
                  disabled={pending || !canUndo}
                  onClick={() => undo(diff.field)}
                >
                  {ui("Undo")} {fieldLabels?.[diff.field] ?? diff.field.replaceAll("_", " ")}
                </button>
              )}
            </div>
            <div className="grid min-w-0 gap-2 sm:grid-cols-2">
              <div className="min-w-0">
                <p className="mb-1 text-xs text-muted-foreground">{ui("Before")}</p>
                <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words border-l-2 border-destructive/50 bg-muted/40 p-2 text-xs">
                  {valueText(diff.from)}
                </pre>
              </div>
              <div className="min-w-0">
                <p className="mb-1 text-xs text-muted-foreground">{ui("After")}</p>
                <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words border-l-2 border-accent bg-accent/5 p-2 text-xs">
                  {valueText(diff.to)}
                </pre>
              </div>
            </div>
          </section>
        ))}
      </div>
    </details>
  );
}
