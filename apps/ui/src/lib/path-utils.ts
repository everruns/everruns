export function basename(value: string): string {
  const clean = value.replace(/\/+$/, "");
  const parts = clean.split("/");
  return parts[parts.length - 1] || clean;
}

/** Display files relative to the primary working directory across host mounts. */
export function relativeFilePath(path: string): string {
  return path.replace(/^\/workspace(?:\/|$)/, "").replace(/^\/+/, "");
}
