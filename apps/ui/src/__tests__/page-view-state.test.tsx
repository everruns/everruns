import { act, render, renderHook } from "@testing-library/react";
import { useState } from "react";
import {
  PageViewEntity,
  PageViewRegistryContext,
  useChatPageFilters,
  useChatPageRows,
  usePageViewRegistry,
} from "@/components/chat-mode/page-view-state";

it("reads live, allowlisted React state and clears entries on unmount or navigation", () => {
  const { result } = renderHook(() => usePageViewRegistry("/models"));
  let change: (value: string) => void;
  function Page() {
    const [search, setSearch] = useState("openai");
    change = setSearch;
    useChatPageFilters({ search, status: "active", ...{ api_key: "fixture-secret" } });
    return <PageViewEntity id="model_1" name="Northstar" />;
  }
  const view = render(
    <PageViewRegistryContext.Provider value={result.current.registry}>
      <Page />
    </PageViewRegistryContext.Provider>,
  );
  expect(result.current.read("/models")).toMatchObject({
    filters: { search: "openai", status: "active" },
    visible_entities: [{ id: "model_1", name: "Northstar" }],
  });
  expect(JSON.stringify(result.current.read("/models"))).not.toContain("fixture-secret");
  act(() => change("new unsaved search"));
  expect(result.current.read("/models").filters.search).toBe("new unsaved search");
  expect(result.current.read("/agents").visible_entities).toEqual([]);
  view.unmount();
  expect(result.current.read("/models").filters).toEqual({});
});

it("limits visible references and excludes values outside the page registry", () => {
  const { result } = renderHook(() => usePageViewRegistry("/agents"));
  render(
    <>
      <PageViewEntity id="chat-transcript-entity" />
      <PageViewRegistryContext.Provider value={result.current.registry}>
        {Array.from({ length: 70 }, (_, index) => (
          <PageViewEntity key={index} id={`agent_${index}`} />
        ))}
      </PageViewRegistryContext.Provider>
    </>,
  );
  const data = result.current.read("/agents");
  expect(data.visible_entities).toHaveLength(50);
  expect(JSON.stringify(data)).not.toContain("chat-transcript");
});

it("bounds explicit visible-row projections and clears data on load, errors, denial and navigation", () => {
  const { result } = renderHook(() => usePageViewRegistry("/durable/queues"));
  function Page({ state = "ready" }: { state?: string }) {
    useChatPageRows(
      "Queue tasks",
      Array.from({ length: 80 }, (_, i) => ({
        id: `task_${i}`,
        status: "pending",
        name: "x".repeat(900),
        ...({ nested: { api_key: "fixture-secret" } } as unknown as Record<string, string>),
      })),
      {
        loading: state === "loading",
        error: state === "error",
        enabled: state !== "denied",
      },
    );
    return null;
  }
  const tree = (state: string) => (
    <PageViewRegistryContext.Provider value={result.current.registry}>
      <Page state={state} />
    </PageViewRegistryContext.Provider>
  );
  const { rerender, unmount } = render(tree("ready"));
  const data = result.current.read("/durable/queues");
  const section = data.sections[0] as { rows: Record<string, unknown>[]; truncated: boolean };
  expect(section.rows.length).toBeLessThanOrEqual(50);
  expect(section.truncated).toBe(true);
  expect(String(section.rows[0].name)).toHaveLength(256);
  expect(JSON.stringify(section)).not.toContain("fixture-secret");
  expect(JSON.stringify(section.rows).length).toBeLessThan(13000);
  expect(result.current.read("/agents").sections).toEqual([]);
  for (const state of ["loading", "error", "denied"]) {
    rerender(tree(state));
    expect(result.current.read("/durable/queues").sections[0]).toMatchObject({ rows: [] });
  }
  unmount();
  expect(result.current.read("/durable/queues").sections).toEqual([]);
});
