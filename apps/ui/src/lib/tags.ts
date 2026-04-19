// Boundary hardening: tolerate malformed or legacy API tag payloads during UI render.
export function normalizeTags(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }

  return value
    .filter((tag): tag is string => typeof tag === "string")
    .map((tag) => tag.trim())
    .filter((tag) => tag.length > 0);
}

export function joinTags(value: unknown): string {
  return normalizeTags(value).join(", ");
}
