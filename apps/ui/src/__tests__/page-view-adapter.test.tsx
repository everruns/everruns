import { QueryClient, QueryClientProvider, QueryObserver } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import { usePageViewAdapter } from "@/components/chat-mode/use-page-view-adapter";

beforeAll(() => {
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
});
it("reads only active authorized resource summaries, preserves UI filters, and rejects writes", () => {
  const client = new QueryClient();
  const observe = (key: string[], data: unknown) =>
    new QueryObserver(client, {
      queryKey: key,
      queryFn: async () => data,
      initialData: data,
      staleTime: Infinity,
    }).subscribe(() => {});
  const off = [
    observe(
      ["models", "org_a"],
      [
        {
          id: "model_1",
          name: "Northstar",
          api_key: "fixture-secret",
          headers: { Authorization: "hidden" },
        },
      ],
    ),
    observe(["models", "org_b"], [{ id: "other-org", name: "Private replacement" }]),
    observe(["providers", "config", "org_a"], { name: "Private config" }),
  ];
  client.setQueryData(["models", "inactive", "org_a"], [{ id: "inactive-record" }]);
  off.push(observe(["models", "failed", "org_a"], [{ name: "Stale failed data" }]));
  client
    .getQueryCache()
    .find({ queryKey: ["models", "failed", "org_a"] })!
    .setState({ status: "error", error: new Error("fixture-error-secret") });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const { result, rerender } = renderHook(
    ({ route }) =>
      usePageViewAdapter({ route, label: "Models" }, "org_a", () => ({
        filters: { search: "Northstar" },
      })),
    { initialProps: { route: "/models" }, wrapper },
  );
  const original = result.current;
  const snapshot = original.read(original.binding!);
  const serialized = JSON.stringify(snapshot);
  expect(snapshot.writable).toBe(false);
  expect(snapshot.values.load_state).toContainEqual(expect.objectContaining({ status: "error" }));
  expect(serialized).toContain("Northstar");
  for (const excluded of [
    "fixture-secret",
    "hidden",
    "other-org",
    "inactive-record",
    "Private config",
    "Stale failed data",
    "fixture-error-secret",
  ])
    expect(serialized).not.toContain(excluded);
  expect(() => original.patch(snapshot.binding, snapshot.revision, { name: "Forbidden" })).toThrow(
    /no active editable/,
  );
  rerender({ route: "/agents" });
  expect(() => original.read(snapshot.binding)).toThrow(/page changed/);
  off.forEach((stop) => stop());
  client.clear();
});
