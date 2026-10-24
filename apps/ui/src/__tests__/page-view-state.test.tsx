import { act, render, renderHook } from "@testing-library/react";
import { useState } from "react";
import {
  PageViewEntity,
  PageViewRegistryContext,
  useChatPageFilters,
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
