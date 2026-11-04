// Whether the agent page offers starter files.
//
// The Files row and the Preview files card are for a filesystem the agent can
// actually use. That is the file_system feature from the agent's capabilities,
// their dependencies, or the harness chain, or starter files that already
// exist (saving those adds the file system capability). Until the catalog and
// harness list have loaded, the section stays hidden so an agent without a
// filesystem does not flash an empty Files card.

import type { Harness } from "@/lib/api/types";
import { resolveHarnessInheritance } from "@/lib/harness-inheritance";

export const FILE_SYSTEM_FEATURE = "file_system";

interface CapabilityFeatures {
  id: string;
  features?: readonly string[] | null;
  dependencies?: readonly string[] | null;
}

export function agentFilesSectionVisible(input: {
  capabilities: ReadonlyArray<{ ref: string }>;
  files: ReadonlyArray<unknown> | null | undefined;
  harnessId: string | null | undefined;
  /** Undefined while the harness list is still loading. */
  harnesses: readonly Harness[] | undefined;
  /** Undefined while the capability catalog is still loading. */
  catalog: readonly CapabilityFeatures[] | undefined;
}): boolean {
  if ((input.files?.length ?? 0) > 0) return true;
  if (!input.catalog || !input.harnesses) return false;

  const catalog = new Map(input.catalog.map((capability) => [capability.id, capability]));
  if (
    refsEnableFileSystem(
      input.capabilities.map((capability) => capability.ref),
      catalog,
    ) !== false
  ) {
    return true;
  }

  const harnessId = input.harnessId?.trim();
  if (!harnessId) return false;

  const byId = new Map(input.harnesses.map((harness) => [harness.id, harness]));
  const harness = byId.get(harnessId);
  // A harness we cannot see might still contribute a filesystem. Keep Files.
  if (!harness) return true;

  const inheritance = resolveHarnessInheritance(harness, byId);
  if (inheritance.hasCycle || inheritance.missingParentId) return true;
  if (inheritance.chain.some((ancestor) => (ancestor.initial_files?.length ?? 0) > 0)) {
    return true;
  }

  const refs = inheritance.chain.flatMap((ancestor) =>
    (ancestor.capabilities ?? []).map((capability) => capability.ref),
  );
  return refsEnableFileSystem(refs, catalog) !== false;
}

/** Preview card: show when the resolved agent has a filesystem, or files to list. */
export function previewShowsFiles(input: {
  features: readonly string[] | null | undefined;
  files: ReadonlyArray<unknown> | null | undefined;
}): boolean {
  if ((input.files?.length ?? 0) > 0) return true;
  // An older preview payload omits features; keep the section rather than guess.
  if (!input.features) return true;
  return input.features.includes(FILE_SYSTEM_FEATURE);
}

// `unknown` means a selected id is missing from the catalog, so Files stays visible.
function refsEnableFileSystem(
  refs: readonly string[],
  catalog: ReadonlyMap<string, CapabilityFeatures>,
): boolean | "unknown" {
  const pending = [...refs];
  const seen = new Set<string>();
  let unknown = false;

  while (pending.length > 0) {
    const id = pending.pop();
    if (!id || seen.has(id)) continue;
    seen.add(id);
    const capability = catalog.get(id);
    if (!capability) {
      unknown = true;
      continue;
    }
    if (capability.features?.includes(FILE_SYSTEM_FEATURE)) return true;
    for (const dependency of capability.dependencies ?? []) pending.push(dependency);
  }

  return unknown ? "unknown" : false;
}
