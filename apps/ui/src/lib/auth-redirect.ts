// Decision: keep login redirect URL generation in one place so middleware and
// the client-side auth fallback preserve identical return_to behavior.
// `return_to` is the single public login-page contract for auth resume — see
// specs/authentication.md. Paths must be relative to the frontend origin.

/** Session storage key for preserving return_to across OAuth and signup flows. */
export const RETURN_TO_STORAGE_KEY = "everruns_return_to";

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

/**
 * Path prefixes that route to the backend (not Next.js). Navigating to one of
 * these via `router.push` would render a 404 instead of hitting the backend
 * handler — they must use full-page navigation (`window.location.assign`) so
 * the reverse proxy / frontend root forwards to the backend route.
 *
 * Includes:
 * - `/oauth/` — MCP OAuth handlers (mounted at server root)
 * - `/api/` — backend mounted under the standard `/api` prefix (default layout)
 * - `/v1/` — backend mounted at the frontend origin root with no API prefix
 *   (used by `build_cli_callback_path` when `frontend_url == base_url`)
 *
 * Keep this in sync with the server's reverse-proxy routing and
 * `crates/server/src/auth/cli_auth.rs::build_cli_callback_path`.
 */
const BACKEND_PATH_PREFIXES = ["/oauth/", "/api/", "/v1/"] as const;

export function isBackendNavigationPath(path: string): boolean {
  return BACKEND_PATH_PREFIXES.some((prefix) => path.startsWith(prefix));
}

function buildAuthPath(
  basePath: "/login" | "/signup",
  returnTo: string | null | undefined,
  email?: string | null,
): string {
  const params = new URLSearchParams();
  if (email) {
    params.set("email", email);
  }
  const sanitized = sanitizeReturnTo(returnTo);
  if (sanitized) {
    params.set("return_to", sanitized);
  }
  const query = params.toString();
  return query ? `${basePath}?${query}` : basePath;
}

/** Build a signup href that preserves a sanitized return_to and optional email. */
export function buildSignupHref(
  returnTo: string | null | undefined,
  email?: string | null,
): string {
  return buildAuthPath("/signup", returnTo, email);
}

/** Build a login href that preserves a sanitized return_to and optional email. */
export function buildLoginHref(returnTo: string | null | undefined, email?: string | null): string {
  return buildAuthPath("/login", returnTo, email);
}

/** Persist a sanitized return_to in sessionStorage for post-auth resume. */
export function persistReturnTo(value: string | null | undefined): void {
  const sanitized = sanitizeReturnTo(value);
  if (!sanitized || typeof sessionStorage === "undefined") {
    return;
  }
  sessionStorage.setItem(RETURN_TO_STORAGE_KEY, sanitized);
}

/** Read and clear a stored return_to from sessionStorage. */
export function consumeReturnTo(): string | null {
  if (typeof sessionStorage === "undefined") {
    return null;
  }
  const stored = sanitizeReturnTo(sessionStorage.getItem(RETURN_TO_STORAGE_KEY));
  sessionStorage.removeItem(RETURN_TO_STORAGE_KEY);
  return stored;
}
