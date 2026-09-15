/**
 * Cached data belongs to the org it was fetched in.
 *
 * Most keys carry the org by convention; the hash makes it structural, so a
 * call site that forgets (or a response that lands after an org switch) cannot
 * put one org's data where another org's is read.
 */
import { dehydrate, hydrate } from "@tanstack/react-query";
import { createAppQueryClient } from "@/lib/query-client";
import { setActiveOrgId } from "@/lib/api/active-org";
import { queryKeys } from "@/lib/query-keys";

beforeEach(() => {
  setActiveOrgId(null);
});

test("the same key holds different data per org", () => {
  const client = createAppQueryClient();

  setActiveOrgId("org_personal");
  client.setQueryData(queryKeys.harnesses.list(false), ["personal-harness"]);
  setActiveOrgId("org_test1");
  client.setQueryData(queryKeys.harnesses.list(false), ["test1-harness"]);

  expect(client.getQueryData(queryKeys.harnesses.list(false))).toEqual(["test1-harness"]);
  setActiveOrgId("org_personal");
  expect(client.getQueryData(queryKeys.harnesses.list(false))).toEqual(["personal-harness"]);
});

test("a key with no org in it still reads cold in another org", () => {
  // The durable lists are org-scoped data under an org-free key.
  const client = createAppQueryClient();

  setActiveOrgId("org_personal");
  client.setQueryData(["durable", "workers"], ["worker-1"]);
  setActiveOrgId("org_test1");

  expect(client.getQueryData(["durable", "workers"])).toBeUndefined();
});

test("hierarchical invalidation still matches on key structure", async () => {
  const client = createAppQueryClient();
  setActiveOrgId("org_test1");
  client.setQueryData(queryKeys.sessions.filtered("org_test1", "threads", 0, 100), []);

  await client.invalidateQueries({ queryKey: queryKeys.sessions.all() });

  expect(
    client.getQueryState(queryKeys.sessions.filtered("org_test1", "threads", 0, 100))
      ?.isInvalidated,
  ).toBe(true);
});

test("a server render's entries hydrate in the org it rendered for", () => {
  // The server hashes under the request's `everruns_org` cookie; the browser
  // publishes that same value before children hydrate. Were the two to
  // disagree, every seeded page would silently refetch.
  const server = createAppQueryClient(() => "org_test1");
  server.setQueryData([...queryKeys.harnesses.list(false), "org_test1"], ["seeded"]);

  const browser = createAppQueryClient();
  setActiveOrgId("org_test1");
  hydrate(browser, dehydrate(server));

  expect(browser.getQueryData([...queryKeys.harnesses.list(false), "org_test1"])).toEqual([
    "seeded",
  ]);
});
