// Labels for memory stores, shared by the Memory list and detail pages.

import type { MemoryEntry, MemoryEntryKind, MemoryStoreView } from "@/lib/api/memory";

export type MemoryStoreGroup = "shared" | "agents" | "yours";

/** Which group a store belongs to on this page. */
export function memoryStoreGroup(store: MemoryStoreView): MemoryStoreGroup {
  if (store.scope === "user") return "yours";
  if (store.scope === "agent") return "agents";
  return "shared";
}

/** A store's display title: its owner for agent and team memories, else its store name. */
export function memoryStoreTitle(store: MemoryStoreView): string {
  if ((store.scope === "agent" || store.scope === "team") && store.owner_name) {
    return store.scope === "team" ? `${store.owner_name} team` : store.owner_name;
  }
  if (store.scope === "user") return "Your memory";
  if (store.store === "org") return "Organization";
  return store.store;
}

/** The kind of store, as a badge. */
export function memoryStoreScopeLabel(store: MemoryStoreView): string {
  switch (store.scope) {
    case "agent":
      return "Agent";
    case "team":
      return "Team";
    case "user":
      return "Private";
    default:
      return store.store === "org" ? "Organization" : "Named";
  }
}

const KIND_ORDER: MemoryEntryKind[] = ["blocked", "decision", "rule", "preference", "fact", "note"];

const KIND_TITLES: Record<MemoryEntryKind, string> = {
  blocked: "Blocked",
  decision: "Decisions",
  rule: "Rules",
  preference: "Preferences",
  fact: "Facts",
  note: "Notes",
};

/** Sections in reading order: open blocked items, proposals, kinds, then resolved items. */
export function groupMemoryEntries(entries: MemoryEntry[]) {
  const sections: { key: string; title: string; entries: MemoryEntry[] }[] = [];
  const push = (key: string, title: string, list: MemoryEntry[]) => {
    if (list.length > 0) sections.push({ key, title, entries: list });
  };
  push(
    "blocked",
    "Blocked",
    entries.filter((e) => e.kind === "blocked" && !e.proposed && !e.resolved_at),
  );
  push(
    "proposals",
    "Proposals to review",
    entries.filter((e) => e.proposed),
  );
  for (const kind of KIND_ORDER.slice(1)) {
    push(
      kind,
      KIND_TITLES[kind],
      entries.filter((e) => e.kind === kind && !e.proposed),
    );
  }
  push(
    "resolved",
    "Resolved",
    entries.filter((e) => e.kind === "blocked" && !e.proposed && !!e.resolved_at),
  );
  return sections;
}
