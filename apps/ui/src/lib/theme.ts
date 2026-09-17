/**
 * Decisions:
 * - Three modes, not a boolean: "system" must stay distinguishable from an
 *   explicit "light" so a user who never chose keeps following their OS.
 * - The choice lives in a cookie, not localStorage, so the server render can
 *   emit the right class on <html> and avoid a light flash on every navigation.
 *   Same reasoning as the `everruns_org` cookie in OrgProvider.
 * - `.dark` on <html> is the only switch; see the `@custom-variant dark` note in
 *   app/design-system.css.
 */

export type ThemeMode = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

export const THEME_COOKIE = "everruns_theme";
export const THEME_MODES: readonly ThemeMode[] = ["light", "dark", "system"];

/** A year: the choice is a preference, not a session detail. */
const THEME_COOKIE_MAX_AGE = 60 * 60 * 24 * 365;

export function parseThemeMode(value: string | null | undefined): ThemeMode {
  return THEME_MODES.includes(value as ThemeMode) ? (value as ThemeMode) : "system";
}

export function persistThemeMode(mode: ThemeMode): void {
  // Re-parse rather than trust the argument: this value is concatenated into a
  // Set-Cookie string, so a caller that ever passed through unvalidated input
  // could otherwise append cookie attributes. The cookie itself holds no
  // secret (a theme name) and must stay readable by THEME_SCRIPT, so it is
  // deliberately not HTTP-only; SameSite=Lax matches the other app cookies.
  const safeMode = parseThemeMode(mode);
  document.cookie = `${THEME_COOKIE}=${safeMode}; path=/; max-age=${THEME_COOKIE_MAX_AGE}; samesite=lax`;
}

export function prefersDark(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  );
}

export function applyTheme(theme: ResolvedTheme): void {
  document.documentElement.classList.toggle("dark", theme === "dark");
}

/**
 * Runs before paint in <head>. The server cannot know the OS preference, so a
 * "system" (or absent) cookie renders light and this corrects it synchronously
 * — without it, dark-mode users get a white flash on every full page load.
 * Inlined as a string because it must execute before React hydrates.
 *
 * The script is a constant: nothing from the request is interpolated into it,
 * and the cookie value it reads is only compared, never evaluated or written
 * to the DOM. The UI document carries no Content-Security-Policy today (the
 * strict CSP in crates/server/src/app_builder.rs covers the API routes Caddy
 * sends to the server, not the pages Next.js serves). Next.js emits its own
 * inline hydration scripts here, so a future `script-src` on this document
 * needs nonces regardless — give this script the same nonce then.
 */
export const THEME_SCRIPT = `(function(){try{var m=document.cookie.match(/(?:^|; )${THEME_COOKIE}=([^;]*)/);var v=m?decodeURIComponent(m[1]):"system";var d=v==="dark"||(v!=="light"&&window.matchMedia("(prefers-color-scheme: dark)").matches);document.documentElement.classList.toggle("dark",d)}catch(e){}})()`;
