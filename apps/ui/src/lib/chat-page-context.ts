/** Share UI location, never arbitrary callback parameters or query-carried credentials. */
export function chatPageRoute(pathname: string, query: Pick<URLSearchParams, "get">): string {
  const context = new URLSearchParams();
  for (const key of ["tab", "sheet", "mode"]) {
    const value = query.get(key);
    if (value) context.set(key, value.slice(0, 80));
  }
  const suffix = context.toString();
  return `${pathname}${suffix ? `?${suffix}` : ""}`;
}
