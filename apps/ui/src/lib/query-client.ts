import { QueryClient, hashKey, type QueryKey } from "@tanstack/react-query";
import { getActiveOrgId } from "@/lib/api/active-org";

/**
 * Cache identity includes the organization the data was fetched in.
 *
 * Most keys carry the org already — `useOrgScopedQuery` appends it, and the
 * session factories take it — but it is a convention every call site has to
 * remember, and the ones that forgot (the durable workers/workflows lists) hold
 * org-scoped data under an org-free key. Folding the org into the key *hash*
 * makes the guarantee structural instead: an entry can only ever be read back
 * in the org it was fetched in, whatever the call site passed.
 *
 * The org comes from the same holder that fills the `X-Org-Id` request header,
 * so cache identity matches the org the request was actually sent with — a
 * response that lands after the user switched orgs files itself under the org
 * it was issued for, and the new org starts from a cold, correct entry.
 *
 * Key *matching* is untouched: `invalidateQueries({ queryKey })` matches on key
 * structure, not on the hash, so hierarchical invalidation behaves as before.
 *
 * `resolveOrgId` is injectable because server-rendered prefetches run outside
 * the browser holder and must hash under the org they rendered for, or the
 * dehydrated entries would not be picked up on hydration (see
 * `lib/server-query.ts`).
 */
export function createAppQueryClient(resolveOrgId: () => string | null = getActiveOrgId) {
  const orgScopedQueryKeyHash = (queryKey: QueryKey): string =>
    `${resolveOrgId() ?? "no-org"}::${hashKey(queryKey)}`;

  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 60 * 1000,
        refetchOnWindowFocus: true,
        retry: 1,
        queryKeyHashFn: orgScopedQueryKeyHash,
      },
    },
  });
}
