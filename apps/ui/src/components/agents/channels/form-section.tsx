"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useState } from "react";
import { ChevronDown } from "lucide-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

/// One titled part of a channel form: a heading and hint on the left, fields on
/// the right. The form is one card of these, never cards inside a card.
export function FormSection({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  const ui = useUiTranslations();
  return (
    <section className="grid gap-4 p-5 md:grid-cols-[13rem_minmax(0,1fr)] md:gap-6 [&+&]:border-t">
      <div>
        <h3 className="text-sm font-semibold">{ui(title)}</h3>
        {hint && <p className="mt-1 text-sm text-muted-foreground">{ui(hint)}</p>}
      </div>
      <div className="min-w-0 space-y-4">{children}</div>
    </section>
  );
}

/// Settings most channels leave at their defaults, folded behind a line that
/// shows the current values so nothing is hidden.
export function FoldedSettings({
  summary,
  children,
}: {
  summary: string;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger asChild>
        <button
          type="button"
          className="flex w-full items-center gap-2 text-left text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ChevronDown
            className={cn(
              "size-4 shrink-0 text-muted-foreground transition-transform",
              !open && "-rotate-90",
            )}
          />
          {summary}
        </button>
      </CollapsibleTrigger>
      <CollapsibleContent className="space-y-4 pt-4">{children}</CollapsibleContent>
    </Collapsible>
  );
}
