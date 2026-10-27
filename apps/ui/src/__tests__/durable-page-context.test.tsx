import { act, render, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ChatModeContext, type ChatModeState } from "@/components/chat-mode/chat-mode-context";
import {
  PageViewRegistryContext,
  usePageViewRegistry,
} from "@/components/chat-mode/page-view-state";
import {
  DurablePageContextProvider,
  useDurablePageRows,
} from "@/components/chat-mode/use-durable-page-rows";
import { OrgContext, type OrgContextValue } from "@/providers/org-provider";
import { queryKeys } from "@/lib/query-keys";

it("requires the global policy and clears cached rows when permission, organization or Adoption changes", () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const key = (org: string) => [...queryKeys.policies.config("durable"), org];
  client.setQueryData(key("org_a"), { policies: { "durable.view": false } });
  const { result } = renderHook(() => usePageViewRegistry("/durable/workers"));
  const mode = { page: { route: "/durable/workers" } } as ChatModeState;
  function Page() {
    useDurablePageRows("Workers", [{ id: "worker_a", status: "active" }]);
    return null;
  }
  const tree = (org: string, adopted = true) => (
    <QueryClientProvider client={client}>
      <OrgContext.Provider
        value={{ currentOrg: { public_id: org, role: "owner" } } as OrgContextValue}
      >
        <ChatModeContext.Provider value={adopted ? mode : null}>
          <PageViewRegistryContext.Provider value={result.current.registry}>
            <DurablePageContextProvider>
              <Page />
            </DurablePageContextProvider>
          </PageViewRegistryContext.Provider>
        </ChatModeContext.Provider>
      </OrgContext.Provider>
    </QueryClientProvider>
  );
  const rows = () => result.current.read("/durable/workers").sections[0];
  const { rerender } = render(tree("org_a"));
  expect(rows()).toMatchObject({ rows: [], load_state: "unavailable" });
  act(() => {
    client.setQueryData(key("org_a"), { policies: { "durable.view": true } });
  });
  // React Query batches notifications; rerender observes the new policy synchronously.
  rerender(tree("org_a"));
  expect(rows()).toMatchObject({ rows: [{ id: "worker_a", status: "active" }] });
  rerender(tree("org_b"));
  expect(rows()).toMatchObject({ rows: [], load_state: "unavailable" });
  rerender(tree("org_a", false));
  expect(rows()).toMatchObject({ rows: [], load_state: "unavailable" });
  client.clear();
});
