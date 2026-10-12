// See knowledge/foundations/code-organization.md (Page Titles) for the format and coverage rules.

export const APP_NAME = "Everruns";
export const TITLE_SEPARATOR = " · ";

export function formatPageTitle(...parts: Array<string | null | undefined>): string {
  const segments = parts
    .map((p) => (typeof p === "string" ? p.trim() : ""))
    .filter((p) => p.length > 0)
    // A section that repeats the specific title ("Chat" · "Chat") adds nothing
    // and burns the left side of a truncated tab.
    .filter((segment, index, all) => index === 0 || segment !== all[index - 1]);
  return [...segments, APP_NAME].join(TITLE_SEPARATOR);
}
