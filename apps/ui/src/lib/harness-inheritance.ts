import type { Harness } from "@/lib/api/types";

export interface HarnessInheritance {
  directParent: Harness | null;
  chain: Harness[];
  missingParentId: string | null;
  hasCycle: boolean;
}

/**
 * Resolve display-only ancestry from the already-loaded harness list. Runtime
 * configuration continues to use the server's canonical effective resolver.
 */
export function resolveHarnessInheritance(
  harness: Harness,
  harnessesById: ReadonlyMap<string, Harness>,
): HarnessInheritance {
  const directParent = harness.parent_harness_id
    ? (harnessesById.get(harness.parent_harness_id) ?? null)
    : null;
  const chain = [harness];
  const visited = new Set([harness.id]);
  let missingParentId: string | null = null;
  let hasCycle = false;
  let parentId = harness.parent_harness_id;

  while (parentId) {
    if (visited.has(parentId)) {
      hasCycle = true;
      break;
    }
    visited.add(parentId);
    const parent = harnessesById.get(parentId);
    if (!parent || parent.status === "deleted") {
      missingParentId = parentId;
      break;
    }
    chain.unshift(parent);
    parentId = parent.parent_harness_id;
  }

  return { directParent, chain, missingParentId, hasCycle };
}
