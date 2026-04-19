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

export function normalizeNotebookRoute(route: string) {
  return route.replace(/\/?$/, "/");
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
