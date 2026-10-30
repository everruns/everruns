// Knowledge states shown with the shared EntityStatus dot and label
// (components/ui/entity-status.tsx, apps/ui/DESIGN.md): sync, indexing and
// extraction are not entity lifecycle, so they map to a tone here. Indexing
// mode is configuration, not a state, and stays a badge where it is shown.

import type { EntityStatusTone } from "@/components/ui/entity-status";
import type { KnowledgeCollection } from "@/lib/api/knowledge";

/** Sentence-case label for a raw server status such as `synced`. */
export function statusLabel(status: string): string {
  const text = status.replaceAll("_", " ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** Tone of a sync or extraction status string reported by the server. */
export function statusTone(status: string | undefined | null): EntityStatusTone {
  switch (status) {
    case "ready":
    case "active":
    case "synced":
    case "succeeded":
    case "indexed":
    case "done":
      return "success";
    case "failed":
    case "error":
      return "destructive";
    case "pending":
    case "running":
    case "syncing":
      return "warning";
    default:
      return "muted";
  }
}

/** The one state worth calling out for a collection, if any. */
export function collectionState(
  collection: KnowledgeCollection,
): { tone: EntityStatusTone; label: string } | null {
  if (collection.jobs?.some((job) => job.status === "failed"))
    return { tone: "destructive", label: "Needs attention" };
  if (collection.sources?.some((source) => source.last_error))
    return { tone: "destructive", label: "Source error" };
  if (collection.indexing !== "off" && collection.revision !== collection.published_revision)
    return { tone: "warning", label: "Indexing" };
  if (collection.jobs?.some((job) => job.status === "pending" || job.status === "running"))
    return { tone: "warning", label: "Processing" };
  return null;
}
