import type { Capability } from "@/lib/api/types";

/** Index a feature-flag object by the capability's flag name. */
export function rolloutFlagMap(flags: object): Readonly<Record<string, boolean | undefined>> {
  return flags as unknown as Readonly<Record<string, boolean | undefined>>;
}

/**
 * Whether a capability can be newly selected.
 *
 * `flags` is null while feature flags are still loading, or when no provider
 * is mounted. In that case rollout-gated capabilities stay selectable; once
 * flags resolve, a capability whose flag is off is hidden from pickers.
 * Screens that already reference it still receive its catalog metadata.
 */
export function capabilitySelectable(
  capability: Pick<Capability, "status" | "feature_flag">,
  flags: Readonly<Record<string, boolean | undefined>> | null,
): boolean {
  if (capability.status !== "available") return false;
  if (!capability.feature_flag || flags == null) return true;
  return flags[capability.feature_flag] === true;
}

/** True when the capability's rollout flag is known to be off. */
export function capabilityTurnedOff(
  capability: Pick<Capability, "feature_flag">,
  flags: Readonly<Record<string, boolean | undefined>> | null,
): boolean {
  if (!capability.feature_flag || flags == null) return false;
  return flags[capability.feature_flag] !== true;
}
