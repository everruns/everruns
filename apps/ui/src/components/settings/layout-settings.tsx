"use client";

import type { ReactNode } from "react";
import { useUiTranslations } from "@/providers/locale-provider";
import { cn } from "@/lib/utils";
import {
  type AppLayout,
  type ClassicChat,
  useClassicChatPreference,
  useLayoutPreference,
} from "@/hooks/use-layout-preference";

/**
 * Personal app layout and, for Classic, how Chat opens (knowledge/ui/chat-mode.md).
 * Both apply immediately and are saved to the account.
 */
export function LayoutSettings() {
  const ui = useUiTranslations();
  const { layout, setLayout, error } = useLayoutPreference();
  const { classicChat, setClassicChat, error: chatError } = useClassicChatPreference();
  const layouts: Array<Option<AppLayout>> = [
    {
      value: "classic",
      label: ui("Classic"),
      description: ui("Navigation on the left and the page fills the rest."),
    },
    {
      value: "chat_first",
      label: ui("Chat first"),
      description: ui(
        "Main chat on the left, the page on the right, and navigation in a rail on the right.",
      ),
    },
  ];
  const chats: Array<Option<ClassicChat>> = [
    {
      value: "full_screen",
      label: ui("Full screen"),
      description: ui("Chat replaces the page. Command-J switches between them."),
    },
    {
      value: "floating",
      label: ui("Floating"),
      description: ui("A card over the page that you can move and resize. Command-J shows it."),
    },
    {
      value: "docked",
      label: ui("Docked"),
      description: ui("A column on the right of the page, with a divider you can drag."),
    },
  ];
  return (
    <section className="space-y-6" aria-label={ui("Layout")}>
      <ChoiceGroup
        id="layout-settings"
        title={ui("Layout")}
        options={layouts}
        value={layout}
        onChange={setLayout}
        preview={(value) => <LayoutPreview layout={value} />}
      />
      <ChoiceGroup
        id="classic-chat-settings"
        title={ui("Chat in Classic")}
        hint={layout === "classic" ? undefined : ui("Applies when the layout is Classic.")}
        options={chats}
        value={classicChat}
        onChange={(value) => {
          setClassicChat(value);
          // Picking how Chat opens in Classic only makes sense in Classic.
          if (layout !== "classic") setLayout("classic");
        }}
        preview={(value) => <ClassicChatPreview chat={value} />}
      />
      <p className="text-sm text-muted-foreground">
        {ui("Your layout follows you across devices and organizations.")}
      </p>
      {(error || chatError) && (
        <p role="alert" className="text-sm text-destructive">
          {ui("Could not save your layout. It applies on this device until you try again.")}
        </p>
      )}
    </section>
  );
}

interface Option<T extends string> {
  value: T;
  label: string;
  description: string;
}

function ChoiceGroup<T extends string>({
  id,
  title,
  hint,
  options,
  value,
  onChange,
  preview,
}: {
  id: string;
  title: string;
  hint?: string;
  options: Array<Option<T>>;
  value: T;
  onChange: (value: T) => void;
  preview: (value: T) => ReactNode;
}) {
  return (
    <div className="space-y-2">
      <h2 id={id} className="text-sm font-medium">
        {title}
      </h2>
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
      <div role="radiogroup" aria-labelledby={id} className="flex flex-wrap gap-3">
        {options.map((option) => {
          const selected = value === option.value;
          return (
            <button
              key={option.value}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => onChange(option.value)}
              className={cn(
                "w-64 border bg-card p-3 text-left transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                selected && "border-primary ring-1 ring-primary",
              )}
            >
              {preview(option.value)}
              <span className="mt-2 block text-sm font-medium">{option.label}</span>
              <span className="mt-1 block text-xs text-muted-foreground">{option.description}</span>
            </button>
          );
        })}
      </div>
    </div>
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

function ClassicChatPreview({ chat }: { chat: ClassicChat }) {
  return (
    <span aria-hidden="true" className="relative flex h-16 gap-1">
      <span className="w-1/5 bg-muted-foreground/30" />
      {chat === "full_screen" ? (
        <span className="flex-1 bg-primary/25" />
      ) : (
        <span className="flex-1 bg-muted" />
      )}
      {chat === "docked" && <span className="w-1/4 bg-primary/25" />}
      {chat === "floating" && (
        <span className="absolute right-1.5 bottom-1.5 h-9 w-1/4 border border-primary/40 bg-primary/25" />
      )}
    </span>
  );
}
