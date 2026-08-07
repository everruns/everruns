// See knowledge/foundations/code-organization.md (Page Titles) for the format and coverage rules.

export const APP_NAME = "Everruns";
export const TITLE_SEPARATOR = " · ";

export function formatPageTitle(...parts: Array<string | null | undefined>): string {
  const segments = parts
    .map((p) => (typeof p === "string" ? p.trim() : ""))
    .filter((p) => p.length > 0);
  return [...segments, APP_NAME].join(TITLE_SEPARATOR);
}
