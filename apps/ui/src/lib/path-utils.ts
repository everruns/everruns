export function basename(value: string): string {
  const clean = value.replace(/\/+$/, "");
  const parts = clean.split("/");
  return parts[parts.length - 1] || clean;
}
