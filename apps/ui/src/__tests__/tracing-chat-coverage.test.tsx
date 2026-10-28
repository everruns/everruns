import { act, render, renderHook, screen } from "@testing-library/react";
import { TracingRoutingForm } from "@/components/tracing/routing-form";
import { useTracingDestinationRows } from "@/components/tracing/use-tracing-page-context";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
import {
  PageViewRegistryContext,
  usePageViewRegistry,
} from "@/components/chat-mode/page-view-state";
import { saveTracingPolicy, type TracingDestination } from "@/lib/api/tracing";

let role = "owner";
let loadState = "ready";
let revision = 1;
const selected = "00000000-0000-4000-8000-000000000001";
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_fixture" }, hasRole: () => role !== "member" }),
}));
jest.mock("@/hooks/use-tracing", () => ({
  useTracingPolicy: (agent?: string) => ({
    data: { revision, policy: { mode: agent ? "inherit" : "replace", destination_ids: [] } },
    isLoading: loadState === "loading",
    error: loadState === "error",
    refetch: jest.fn(),
  }),
  useTracingDestinations: () => ({ data: [], isLoading: false, error: null, refetch: jest.fn() }),
  useRefreshTracing: () => jest.fn(),
}));
jest.mock("@/lib/api/tracing", () => ({
  ...jest.requireActual("@/lib/api/tracing"),
  saveTracingPolicy: jest.fn(),
}));
jest.mock("@/components/entity-actions/history-sheet", () => ({ HistorySheet: () => null }));
let adapter: PageDraftAdapter;
let priority = -1;
const mode: ChatModeState = {
  page: { route: "/settings/tracing", label: "Tracing" },
  pageContainer: { current: null },
  open: true,
  setOpen: () => {},
  actions: [],
  recordAction: () => {},
  registerPage: (_, value, rank = 0) => {
    if (value && rank >= priority) {
      adapter = value;
      priority = rank;
    }
    return () => {};
  },
};
const read = () => adapter.read(adapter.describe().binding);
beforeEach(() => {
  role = "owner";
  loadState = "ready";
  revision = 1;
  priority = -1;
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
  jest.clearAllMocks();
});

it.each(["owner", "admin", "member"])(
  "guards %s routing edits and never saves a Chat patch",
  (viewer) => {
    role = viewer;
    render(
      <PageChatProvider value={mode}>
        <TracingRoutingForm agentId="agent_fixture" />
      </PageChatProvider>,
    );
    const before = read();
    if (viewer === "member") {
      expect(before.writable).toBe(false);
      expect(() => adapter.patch(before.binding, before.revision, { mode: "off" })).toThrow();
    } else {
      act(() => {
        adapter.patch(before.binding, before.revision, {
          mode: "replace",
          destination_ids: [selected],
        });
      });
      expect(read().values).toEqual({ mode: "replace", destination_ids: [selected] });
      expect(screen.getByText("Draft edited by Chat")).toBeVisible();
    }
    expect(saveTracingPolicy).not.toHaveBeenCalled();
  },
);

it("keeps the organization-only routing mode read-only and rejects a mixed patch atomically", () => {
  render(
    <PageChatProvider value={mode}>
      <TracingRoutingForm />
    </PageChatProvider>,
  );
  const before = read();
  expect(() =>
    adapter.patch(before.binding, before.revision, { mode: "off", destination_ids: [selected] }),
  ).toThrow();
  expect(read().values).toEqual(before.values);
  act(() => {
    adapter.patch(before.binding, before.revision, { destination_ids: [selected] });
  });
  expect(read().values).toEqual({ mode: "replace", destination_ids: [selected] });
});

it("refuses a stale routing save and preserves the dirty draft", () => {
  const tree = (
    <PageChatProvider value={mode}>
      <TracingRoutingForm agentId="agent_fixture" />
    </PageChatProvider>
  );
  const view = render(tree);
  const before = read();
  act(() => {
    adapter.patch(before.binding, before.revision, { mode: "off" });
  });
  revision = 2;
  view.rerender(
    <PageChatProvider value={mode}>
      <TracingRoutingForm agentId="agent_fixture" />
    </PageChatProvider>,
  );
  expect(read().values.mode).toBe("off");
  expect(read().writable).toBe(false);
  expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
});

it.each(["loading", "error"])(
  "masks underlying editors when tracing is %s, including cached data",
  (state) => {
    loadState = state;
    render(
      <PageChatProvider value={mode}>
        <TracingRoutingForm agentId="agent_fixture" />
      </PageChatProvider>,
    );
    const before = read();
    expect(before.writable).toBe(false);
    expect(before.values.interaction).toContain("manual controls");
    expect(() => adapter.patch(before.binding, before.revision, { mode: "off" })).toThrow();
  },
);

it.each(["ready", "loading", "error"])(
  "projects bounded tracing rows in %s state without endpoints or raw errors",
  (state) => {
    const registry = renderHook(() => usePageViewRegistry("/settings/tracing"));
    function Rows() {
      useTracingDestinationRows(
        [
          {
            id: selected,
            revision: 1,
            config: {
              name: "Fictional receiver",
              provider: "otlp",
              endpoint: "https://fixture-secret@example.test",
              project_id: "fixture-secret",
              capture_content: false,
              capture_reasoning: false,
              enabled: false,
            },
            last_error: "fixture-secret",
            last_accepted_at: null,
            last_attempt_at: null,
            dropped_events: 1,
            organization_default: false,
            agent_selections: 0,
          } satisfies TracingDestination,
        ],
        { loading: state === "loading", error: state === "error" },
      );
      return null;
    }
    const view = render(
      <PageViewRegistryContext.Provider value={registry.result.current.registry}>
        <Rows />
      </PageViewRegistryContext.Provider>,
    );
    const context = registry.result.current.read("/settings/tracing");
    expect(JSON.stringify(context)).not.toContain("fixture-secret");
    expect(context.sections[0]).toMatchObject({ load_state: state });
    if (state === "ready")
      expect(context.sections[0]).toMatchObject({
        rows: [{ id: selected, name: "Fictional receiver", has_error: true }],
      });
    else expect(context.sections[0]).toMatchObject({ rows: [] });
    view.unmount();
    expect(registry.result.current.read("/settings/tracing").sections).toEqual([]);
  },
);
