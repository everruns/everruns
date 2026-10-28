import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { QueryClient, QueryClientProvider, QueryObserver } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import cases from "@/test-utils/chat-page-coverage.json";
import { pageLocation } from "@/lib/chat-page-view";
import { usePageViewAdapter } from "@/components/chat-mode/use-page-view-adapter";

const root = path.join(process.cwd(), "src/app/(main)");
function pages(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory()
      ? pages(file)
      : entry.name === "page.tsx"
        ? [path.relative(root, file).replaceAll(path.sep, "/")]
        : [];
  });
}
const inventory = Object.entries(cases).flatMap(([kind, sources]) =>
  sources.map((source) => ({ kind, source })),
);
const resourceId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
function publicRoute(source: string) {
  return (
    "/" +
    source
      .split("/")
      .filter((part) => !part.startsWith("(") && part !== "page.tsx")
      .map((part) => (part.startsWith("[") ? `fixture_${resourceId}` : part))
      .join("/")
  );
}

beforeAll(() => {
  global.structuredClone = (v) => JSON.parse(JSON.stringify(v));
});

// A new/moved route needs an explicit coverage decision. This is a catalog and
// fallback-contract gate, not a substitute for rendered-editor/browser tests.
it("classifies every authenticated page exactly once, including new UIs", () => {
  const declared = inventory.map((entry) => entry.source);
  expect(declared.length).toBe(new Set(declared).size);
  expect(declared.sort()).toEqual(pages(root).sort());
});

it.each(inventory.filter((entry) => entry.kind === "redirect"))(
  "keeps $source as a redirect with no independent Chat",
  ({ source }) => {
    expect(readFileSync(path.join(root, source), "utf8")).toMatch(/\bredirect\(/);
  },
);

it.each(inventory.filter((entry) => !["redirect", "conversation"].includes(entry.kind)))(
  "binds $source to scoped context and refuses writes without an active editor",
  ({ source }) => {
    const route = publicRoute(source);
    const page = pageLocation(route);
    expect(page.entity_type).toBeDefined();
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const stop = new QueryObserver(client, {
      queryKey: ["agents", "another_org"],
      queryFn: async () => [],
      staleTime: Infinity,
      initialData: [{ name: "other-tenant-private-record", api_key: "fixture-secret" }],
    }).subscribe(() => {});
    const hook = renderHook(
      ({ route }) =>
        usePageViewAdapter(pageLocation(route), "org_fixture", () => ({
          filters: { search: "Current UI state" },
        })),
      {
        initialProps: { route },
        wrapper: ({ children }) => (
          <QueryClientProvider client={client}>{children}</QueryClientProvider>
        ),
      },
    );
    const adapter = hook.result.current;
    const before = adapter.read(adapter.describe().binding);
    expect(before).toMatchObject({
      writable: false,
      dirty: false,
      values: {
        page,
        organization_id: "org_fixture",
        view: { filters: { search: "Current UI state" } },
      },
    });
    expect(JSON.stringify(before)).not.toContain("other-tenant-private-record");
    expect(JSON.stringify(before)).not.toContain("fixture-secret");
    expect(() =>
      adapter.patch(before.binding, before.revision, { name: "Unintended change" }),
    ).toThrow(/no active editable form/);
    expect(() => adapter.read("another-browser")).toThrow(/page changed/);
    hook.rerender({ route: "/chats" });
    expect(() => adapter.read(before.binding)).toThrow(/page changed/);
    hook.unmount();
    stop();
    client.clear();
  },
);

it.each([
  "/durable/workers",
  "/durable/workflows",
  "/durable/queues",
  "/durable/schedules",
  "/durable/circuit-breakers",
  "/system/health",
  "/knowledge/accounts",
])("does not mistake a fixed destination for a resource: %s", (route) => {
  expect(pageLocation(route).entity_id).toBeUndefined();
});
