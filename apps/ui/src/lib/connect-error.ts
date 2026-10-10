// A browser OAuth connect that fails comes back to the page it started from
// with `?connect_error=<code>&provider=<provider>` (or, in a popup, through
// /connection-complete with `status=error`). The server keeps the detail in
// its log; the console turns the code into one plain sentence.
import { useEffect, useState } from "react";

export type ConnectErrorCode =
  | "blocked_by_network_policy"
  | "provider_unreachable"
  | "provider_refused"
  | "failed";

export function connectErrorMessage(code: string | null | undefined): string {
  switch (code) {
    case "blocked_by_network_policy":
      return "Couldn't connect: this server's host isn't on the organization's allowed network list.";
    case "provider_unreachable":
      return "Couldn't reach the server's sign-in service. Try again later.";
    case "provider_refused":
      return "The provider declined the sign-in.";
    default:
      return "Couldn't connect. Try again.";
  }
}

const CONNECT_RESULT_PARAMS = ["connect_error", "provider", "connected"];

/** Drop the connect result from the address bar, keeping every other param. */
export function stripConnectResultParams(): void {
  const url = new URL(window.location.href);
  for (const key of CONNECT_RESULT_PARAMS) url.searchParams.delete(key);
  window.history.replaceState(window.history.state, "", `${url.pathname}${url.search}${url.hash}`);
}

/**
 * Read a `connect_error` the server sent this page back with, once, and strip
 * it from the URL. Returns the message and a dismiss callback.
 */
export function useConnectErrorFromUrl(): [string | null, () => void] {
  const [message, setMessage] = useState<string | null>(null);
  useEffect(() => {
    const code = new URLSearchParams(window.location.search).get("connect_error");
    if (!code) return;
    setMessage(connectErrorMessage(code));
    stripConnectResultParams();
  }, []);
  return [message, () => setMessage(null)];
}
