import manifest from "../generated/notebooks/manifest.json";

export interface NotebookTocItem {
  depth: number;
  slug: string;
  text: string;
  children: NotebookTocItem[];
}

export interface NotebookToc {
  items: NotebookTocItem[];
  minHeadingLevel: number;
  maxHeadingLevel: number;
}

export interface NotebookEntry {
  htmlFile: string;
  publicHref: string;
  route: string;
  docPage: string;
  notebookFile: string;
  toc?: NotebookToc;
}

const notebookManifest = manifest as Record<string, NotebookEntry>;

// Manifest routes are base-less (`/tutorials/x/`); page URLs carry the base.
export function normalizeNotebookRoute(route: string) {
  const base = import.meta.env.BASE_URL.replace(/\/+$/, "");
  const withoutBase = base && route.startsWith(`${base}/`) ? route.slice(base.length) : route;
  return withoutBase.replace(/\/?$/, "/");
}

export function findNotebookEntry({
  route,
  source,
}: {
  route: string;
  source?: string;
}) {
  const normalizedRoute = normalizeNotebookRoute(route);

  return Object.entries(notebookManifest).find(([entrySource, entry]) =>
    typeof source === "string" && source.length > 0
      ? entrySource === source
      : entry.route === normalizedRoute
  )?.[1];
}
