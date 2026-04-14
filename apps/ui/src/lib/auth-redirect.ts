// Decision: keep login redirect URL generation in one place so middleware and
// the client-side auth fallback preserve identical return_to behavior.

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
