// Boundary hardening: tolerate malformed or legacy API tag payloads during UI render.
export function normalizeTags(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }

  const tags = value
    .filter((tag): tag is string => typeof tag === "string")
    .map((tag) => tag.trim())
    .filter((tag) => tag.length > 0);

  // A JSON array copied into a comma-separated flag is stored as the pieces
  // `["company"` and `"ci"]`, or as the whole text `["company", "ci"]`. Either
  // rejoins into the array the caller meant.
  const restored = parseJsonStringList(tags.join(", "));
  return restored ?? tags;
}

/** A JSON array of strings, or null when `value` is not one. */
export function parseJsonStringList(value: string): string[] | null {
  const trimmed = value.trim();
  if (!trimmed.startsWith("[")) return null;

  try {
    const parsed = JSON.parse(trimmed) as unknown;
    if (!Array.isArray(parsed) || !parsed.every((item) => typeof item === "string")) {
      return null;
    }
    return parsed.map((item) => item.trim()).filter((item) => item.length > 0);
  } catch {
    return null;
  }
}

export function joinTags(value: unknown): string {
  return normalizeTags(value).join(", ");
}
