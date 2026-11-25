/** Each chat has a distinct URL and API prefix. Selection is never a cookie:
 * two tabs can work in different chats without changing each other's target. */
export function apiPath(path: string): string {
  if (path === "chat/profile" || path.startsWith("virtual-users/")) return `/api/${path}`;
  const selected =
    typeof window !== "undefined"
      ? window.location.pathname.match(/^\/chats\/([a-z][a-z0-9-]{1,46}[a-z0-9])\/?$/)?.[1]
      : undefined;
  return `/api/${selected ? `chats/${selected}/` : ""}${path}`;
}
