// Section tabs on registry pages (Models, Web search, Sandboxes) are real
// paths (`/models/providers`), not `?tab=` query params, so a tab is a link
// that can be shared, opened in a new tab and kept in history. Old `?tab=`
// links still land on the right tab through `legacyTabRedirect`.

import { redirect } from "next/navigation";

export type SearchParamsRecord = Record<string, string | string[] | undefined>;

/**
 * The path for `tab` under `base`; the default tab is `base` itself.
 * Other query params are dropped: they belong to the tab being left.
 */
export function tabHref(base: string, tab: string, defaultTab: string): string {
  return tab === defaultTab ? base : `${base}/${tab}`;
}

/** The tab a pathname under `base` names, or `defaultTab` for `base` itself. */
export function tabFromPath<T extends string>(
  pathname: string | null,
  base: string,
  tabs: readonly T[],
  defaultTab: T,
): T {
  const rest = (pathname ?? "").slice(base.length).split("/")[1] ?? "";
  return (tabs as readonly string[]).includes(rest) ? (rest as T) : defaultTab;
}

/**
 * Redirects a retired `?tab=<name>` link to the tab's own path, keeping every
 * other query param. Unknown tab names are dropped and land on the default tab.
 */
export function legacyTabRedirect(
  base: string,
  searchParams: SearchParamsRecord,
  tabs: readonly string[],
  defaultTab: string,
): void {
  const tab = searchParams.tab;
  if (tab === undefined) return;
  const name = Array.isArray(tab) ? tab[0] : tab;
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(searchParams)) {
    if (key === "tab" || value === undefined) continue;
    for (const item of Array.isArray(value) ? value : [value]) params.append(key, item);
  }
  const path = name && tabs.includes(name) ? tabHref(base, name, defaultTab) : base;
  redirect(params.size ? `${path}?${params}` : path);
}
