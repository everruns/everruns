// Shared formatting helpers for knowledge-index sync status.

import type { KnowledgeIndexSyncStatus } from "@/lib/api/types";

type BadgeVariant = "default" | "secondary" | "destructive" | "outline" | "accent";

// Maps the Syncout pipeline state to a badge variant. `synced` is the healthy
// terminal state, `failed` is destructive, in-flight states use accent, and
// `idle` is neutral.
export function syncStatusBadgeVariant(status: KnowledgeIndexSyncStatus): BadgeVariant {
  switch (status) {
    case "synced":
      return "default";
    case "failed":
      return "destructive";
    case "pending":
    case "syncing":
      return "accent";
    case "idle":
    default:
      return "secondary";
  }
}
