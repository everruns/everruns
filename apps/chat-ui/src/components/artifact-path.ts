/** Resolve Markdown artifact URLs once; workspace authorization stays on the API. */
export function artifactPath(href: string | undefined): string | null {
  if (!href || !(href.startsWith("/workspace/") || href.startsWith("/attachments/"))) return null;
  try {
    const path = decodeURIComponent(new URL(href, "https://artifact.invalid").pathname);
    return path.startsWith("/workspace/") || path.startsWith("/attachments/") ? path : null;
  } catch {
    return null;
  }
}
