import type { Harness } from "@/lib/api/types";

/** Deprecation is built-in API metadata, independent of lifecycle status. */
export function isHarnessDeprecated(harness: Pick<Harness, "is_built_in" | "tags">): boolean {
  return harness.is_built_in && (harness.tags ?? []).includes("deprecated");
}

export function harnessChoiceLabel(name: string): string {
  const labels: Record<string, string> = {
    base: "Base",
    conversation: "Conversation",
    "worker-base": "Worker Base",
    worker: "Worker",
  };
  return labels[name] ?? name;
}
