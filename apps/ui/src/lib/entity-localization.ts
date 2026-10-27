// Display overlays never replace stable resource names or execution instructions.
export interface EntityLocalization {
  display_name?: string | null;
  description?: string | null;
}
export type EntityLocalizations = Record<string, EntityLocalization>;

export function localeCandidates(locale: string = "en"): string[] {
  const normalized = locale.trim().replace(/_/g, "-").toLowerCase();
  return [...new Set([normalized, normalized.split("-")[0], "en"])];
}

export function localizedEntityField(
  entity: { localizations?: EntityLocalizations | null },
  field: keyof EntityLocalization,
  locale: string,
): string | undefined {
  const entries = entity.localizations;
  for (const candidate of localeCandidates(locale)) {
    const key = Object.keys(entries ?? {}).find((key) => key.toLowerCase() === candidate);
    const value = key ? entries?.[key]?.[field] : undefined;
    if (value?.trim()) return value;
  }
}

export function localizedEntityDescription(
  entity: { description?: string | null; localizations?: EntityLocalizations | null },
  locale: string,
): string {
  return localizedEntityField(entity, "description", locale) ?? entity.description ?? "";
}
