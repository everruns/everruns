"use client";

/**
 * Dev page: Saved changes bar
 *
 * The bar that floats over the bottom of a page after a save, in each state:
 * collapsed, Review open with per-field Undo, the Undid confirmation, and a
 * resource that changed again since the save.
 */

import type { ReactNode } from "react";
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import type { SavedPageChange } from "@/components/chat-mode/page-change-review-controller";
import { SavedChangesBar, UndoneBar } from "@/components/chat-mode/saved-changes-bar";

const resource = { kind: "model", id: "model_dev", label: "GPT-6.1 Sol" };
const receipt: SavedPageChange = {
  resource,
  from: 3,
  revision: 4,
  stale: false,
  pending: false,
  fields: [
    { field: "display_name", from: "GPT-6.1 Sol", to: "GPT-6.1 Sol (fast)" },
    { field: "enabled", from: false, to: true },
    {
      field: "capabilities",
      from: ["chat", "tools"],
      to: ["chat", "tools", "vision", "thinking"],
    },
  ],
};
const noop = () => {};

function Frame({ title, children }: { title: string; children: ReactNode }) {
  return (
    <figure className="space-y-2">
      <figcaption className="text-sm font-medium">{title}</figcaption>
      <div className="relative h-80 overflow-hidden border bg-background">
        <div className="border-b bg-card px-4 py-3">
          <p className="font-medium">GPT-6.1 Sol</p>
          <p className="text-xs text-muted-foreground">Model · OpenAI</p>
        </div>
        <div className="grid gap-3 p-4 text-sm">
          {[
            ["Display name", "GPT-6.1 Sol (fast)"],
            ["Model id", "gpt-6.1-sol"],
          ].map(([label, value]) => (
            <div key={label} className="grid grid-cols-[8rem_1fr] items-center gap-3">
              <span className="text-muted-foreground">{label}</span>
              <span className="border bg-card px-2 py-1">{value}</span>
            </div>
          ))}
        </div>
        <div className="pointer-events-none absolute inset-x-0 bottom-4 flex flex-col items-center gap-2 px-4">
          {children}
        </div>
      </div>
    </figure>
  );
}

export default function SavedChangesDevPage() {
  return (
    <DevPageShell
      eyebrow="Pages"
      title="Saved changes"
      description="After a save, a small bar at the bottom of the page offers Review and Undo."
    >
      <div className="grid gap-6 lg:grid-cols-2">
        <Frame title="After saving">
          <SavedChangesBar
            receipt={receipt}
            dirty={false}
            onUndo={noop}
            onDismiss={noop}
            onHistory={noop}
          />
        </Frame>
        <Frame title="Review open">
          <SavedChangesBar
            receipt={receipt}
            dirty={false}
            initiallyOpen
            onUndo={noop}
            onDismiss={noop}
            onHistory={noop}
          />
        </Frame>
        <Frame title="After Undo all">
          <UndoneBar count={3} resource={resource} onHistory={noop} onDone={noop} />
        </Frame>
        <Frame title="Changed again since the save">
          <SavedChangesBar
            receipt={{ ...receipt, stale: true }}
            dirty={false}
            onUndo={noop}
            onDismiss={noop}
            onHistory={noop}
          />
        </Frame>
      </div>
    </DevPageShell>
  );
}
