// Client-side resolution of capability localizations.
//
// The backend ships English base strings on Capability (name, description,
// config_schema titles) plus per-locale overlays in `localizations`. The
// client owns picking the locale and merging the overlay into the schema
// before rendering, so the generic settings editor stays capability-agnostic.

import type { Capability, CapabilityLocalization } from "@/lib/api/types";
import { localeCandidates } from "@/lib/entity-localization";

function localizedField<K extends keyof CapabilityLocalization>(
  capability: Pick<Capability, "localizations">,
  locale: string,
  field: K,
): CapabilityLocalization[K] | undefined {
  for (const candidate of localeCandidates(locale)) {
    const key = Object.keys(capability.localizations ?? {}).find(
      (key) => key.toLowerCase() === candidate,
    );
    const value = key ? capability.localizations?.[key]?.[field] : undefined;
    if (value !== undefined && value !== null) return value;
  }
}

export function localizedCapabilityName(
  capability: Pick<Capability, "name" | "localizations">,
  locale: string,
): string {
  return localizedField(capability, locale, "name") ?? capability.name;
}

export function localizedCapabilityDescription(
  capability: Pick<Capability, "description" | "localizations">,
  locale: string,
): string {
  return localizedField(capability, locale, "description") ?? capability.description;
}

/** First paragraph, for a list row. The capability page keeps the full text. */
export function capabilitySummary(description: string): string {
  const paragraph = description.split(/\n\s*\n/)[0] ?? description;
  return paragraph.replace(/\s+/g, " ").trim();
}

/** One-line summary of what the capability's config controls, if any. */
export function localizedConfigDescription(
  capability: Pick<Capability, "localizations">,
  locale: string,
): string | undefined {
  return (
    localizedField(capability, locale, "config_description") ??
    capability.localizations?.en?.config_description
  );
}

interface OverlayNode {
  title?: string;
  description?: string;
  /** Map from enum const value to localized option label */
  enum_labels?: Record<string, string>;
  properties?: Record<string, OverlayNode>;
  items?: OverlayNode;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function mergeNode(schema: Record<string, unknown>, overlay: OverlayNode): Record<string, unknown> {
  const merged: Record<string, unknown> = { ...schema };

  if (overlay.title) merged.title = overlay.title;
  if (overlay.description) merged.description = overlay.description;

  if (overlay.enum_labels) {
    const labels = overlay.enum_labels;
    if (Array.isArray(merged.oneOf)) {
      merged.oneOf = merged.oneOf.map((option) =>
        isRecord(option) && "const" in option && labels[String(option.const)] !== undefined
          ? { ...option, title: labels[String(option.const)] }
          : option,
      );
    } else if (Array.isArray(merged.enum)) {
      // Schemas should prefer oneOf const/title for labeled enums; rewrite
      // plain enums so localized labels still render.
      merged.oneOf = merged.enum.map((value) => ({
        const: value,
        title: labels[String(value)] ?? String(value),
      }));
      delete merged.enum;
    }
  }

  if (overlay.properties && isRecord(merged.properties)) {
    const properties: Record<string, unknown> = { ...merged.properties };
    for (const [key, child] of Object.entries(overlay.properties)) {
      const target = properties[key];
      if (isRecord(target)) {
        properties[key] = mergeNode(target, child);
      }
    }
    merged.properties = properties;
  }

  if (overlay.items && isRecord(merged.items)) {
    merged.items = mergeNode(merged.items, overlay.items);
  }

  return merged;
}

/**
 * Returns config_schema with the locale's overlay merged in
 * (titles, descriptions, enum option labels). Unknown overlay paths are
 * ignored. Validation semantics are preserved: the only structural change
 * is rewriting a plain `enum` into the equivalent labeled `oneOf`
 * const/title list when the overlay provides `enum_labels`.
 */
export function localizedConfigSchema(
  capability: Pick<Capability, "config_schema" | "localizations">,
  locale: string,
): Record<string, unknown> | undefined {
  const schema = capability.config_schema;
  if (!schema) return undefined;
  return localeCandidates(locale)
    .reverse()
    .reduce((result, candidate) => {
      const key = Object.keys(capability.localizations ?? {}).find(
        (key) => key.toLowerCase() === candidate,
      );
      const overlay = key
        ? (capability.localizations?.[key]?.config_overlay as OverlayNode | undefined)
        : undefined;
      return overlay ? mergeNode(result, overlay) : result;
    }, schema);
}
