"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { cn } from "@/lib/utils";
import { type AppLayout, useLayoutPreference } from "@/hooks/use-layout-preference";

/** Personal app layout (knowledge/ui/chat-mode.md). Applies immediately. */
export function LayoutSettings() {
  const ui = useUiTranslations();
  const { layout, setLayout, error } = useLayoutPreference();
  const options: Array<{ value: AppLayout; label: string; description: string }> = [
    {
      value: "chat_first",
      label: ui("Chat first"),
      description: ui(
        "Main chat on the left, the page on the right, and navigation in a rail on the right.",
      ),
    },
    {
      value: "classic",
      label: ui("Classic"),
      description: ui(
        "Navigation on the left and the page fills the rest. Chat opens full screen.",
      ),
    },
  ];
  return (
    <section className="space-y-2" aria-labelledby="layout-settings">
      <h2 id="layout-settings" className="text-sm font-medium">
        {ui("Layout")}
      </h2>
      <div role="radiogroup" aria-labelledby="layout-settings" className="flex flex-wrap gap-3">
        {options.map((option) => {
          const selected = layout === option.value;
          return (
            <button
              key={option.value}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => setLayout(option.value)}
              className={cn(
                "w-64 border bg-card p-3 text-left transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                selected && "border-primary ring-1 ring-primary",
              )}
            >
              <LayoutPreview layout={option.value} />
              <span className="mt-2 block text-sm font-medium">{option.label}</span>
              <span className="mt-1 block text-xs text-muted-foreground">{option.description}</span>
            </button>
          );
        })}
      </div>
      <p className="text-sm text-muted-foreground">
        {ui("Your layout follows you across devices and organizations.")}
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {ui("Could not save your layout. It applies on this device until you try again.")}
        </p>
      )}
    </section>
  );
}

function LayoutPreview({ layout }: { layout: AppLayout }) {
  return (
    <span aria-hidden="true" className="flex h-16 gap-1">
      {layout === "chat_first" ? (
        <>
          <span className="w-2/5 bg-muted-foreground/30" />
          <span className="flex-1 bg-muted" />
          <span className="w-2 bg-muted-foreground/30" />
        </>
      ) : (
        <>
          <span className="w-1/5 bg-muted-foreground/30" />
          <span className="flex-1 bg-muted" />
        </>
      )}
    </span>
  );
}
