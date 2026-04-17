// Decision: keep login redirect URL generation in one place so middleware and
// the client-side auth fallback preserve identical return_to behavior.
// `return_to` is the single public login-page contract for auth resume — see
// specs/authentication.md. Paths must be relative to the frontend origin.

type SearchInput = URLSearchParams | string | null | undefined;

function getSearchString(search: SearchInput): string {
  if (search instanceof URLSearchParams) {
    return search.toString();
  }

  if (typeof search === "string") {
    return search.startsWith("?") ? search.slice(1) : search;
  }

  return "";
}

export function getLoginRedirectPath(pathname: string, search: SearchInput): string {
  const searchString = getSearchString(search);
  const currentUrl = pathname + (searchString ? `?${searchString}` : "");

  if (currentUrl === "/dashboard") {
    return "/login";
  }

  return `/login?return_to=${encodeURIComponent(currentUrl)}`;
}

/**
 * Validate a `return_to` value is a safe relative path on the frontend origin.
 *
 * Rejects absolute URLs, protocol-relative URLs (`//evil.com/...`), and any
 * value that doesn't start with `/`. Returns the sanitized path or `null` if
 * invalid. The login page must use this to guard against open-redirect into
 * attacker-controlled origins.
 */
export function sanitizeReturnTo(value: string | null | undefined): string | null {
  if (!value) return null;
  if (!value.startsWith("/")) return null;
  if (value.startsWith("//")) return null;
  if (value.startsWith("/\\")) return null;
  return value;
}
