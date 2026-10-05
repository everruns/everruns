import type { AvatarPreset } from "@/lib/api/agents";

/** Every whitespace-separated term may match any presentation field. */
export function searchAvatarPresets(
  presets: AvatarPreset[],
  query: string,
  family = "",
): AvatarPreset[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  return presets.filter((preset) => {
    if (family && preset.family !== family) return false;
    const text = [preset.name, preset.description, preset.family, preset.role, ...preset.keywords]
      .join(" ")
      .toLowerCase();
    return terms.every((term) => text.includes(term));
  });
}
