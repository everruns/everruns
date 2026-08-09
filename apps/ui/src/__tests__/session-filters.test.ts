// The sessions page keeps its whole filter set in the URL (EVE-853). These
// tests pin the two properties that rests on: a filter set survives a round
// trip through a link, and any change to what is counted resets the page.

import {
  EMPTY_SESSION_FILTERS,
  hasActiveFilters,
  parseSessionFilters,
  serializeSessionFilters,
  toQueryParams,
  toggleAgentFilter,
  toggleFilterValue,
  windowStart,
  withFilterChange,
  type SessionFilters,
} from "@/lib/session-filters";
import { isViewActive, parseSavedViews } from "@/lib/session-views";

const filters = (overrides: Partial<SessionFilters> = {}): SessionFilters => ({
  ...EMPTY_SESSION_FILTERS,
  ...overrides,
});

describe("session filter URL round trip", () => {
  it("omits defaults so a pristine list has a clean URL", () => {
    expect(serializeSessionFilters(EMPTY_SESSION_FILTERS)).toBe("");
  });

  it("round-trips a full filter set", () => {
    const original = filters({
      search: "churn by plan",
      status: ["failed", "running"],
      source: ["slack"],
      agent: "agent_123",
      window: "7d",
      mine: true,
      page: 3,
    });
    const restored = parseSessionFilters(new URLSearchParams(serializeSessionFilters(original)));
    expect(restored).toEqual(original);
  });

  it("falls back on unknown values instead of filtering on nonsense", () => {
    const parsed = parseSessionFilters(new URLSearchParams("window=forever&page=-2&mine=maybe"));
    expect(parsed.window).toBe("all");
    expect(parsed.page).toBe(0);
    expect(parsed.mine).toBe(false);
  });

  it("deduplicates repeated facet values", () => {
    expect(parseSessionFilters(new URLSearchParams("status=failed,failed")).status).toEqual([
      "failed",
    ]);
  });
});

describe("pagination consistency", () => {
  it("resets to page 1 when a facet is toggled", () => {
    const next = toggleFilterValue(filters({ page: 4 }), "source", "slack");
    expect(next.source).toEqual(["slack"]);
    expect(next.page).toBe(0);
  });

  it("resets to page 1 when an agent is selected, and clears on re-click", () => {
    const selected = toggleAgentFilter(filters({ page: 2 }), "agent_1");
    expect(selected).toMatchObject({ agent: "agent_1", page: 0 });
    expect(toggleAgentFilter(selected, "agent_1").agent).toBeNull();
  });

  it("keeps the page when the page itself is what changed", () => {
    expect(withFilterChange(filters({ status: ["failed"] }), { page: 2 }).page).toBe(2);
  });

  it("resets the page for every other change", () => {
    expect(withFilterChange(filters({ page: 5 }), { search: "x" }).page).toBe(0);
  });
});

describe("API parameters", () => {
  it("sends nothing for an unfiltered list", () => {
    expect(toQueryParams(EMPTY_SESSION_FILTERS)).toEqual({
      search: undefined,
      status: undefined,
      source: undefined,
      agentId: undefined,
      mine: undefined,
      createdAfter: undefined,
    });
  });

  it("joins multi-select dimensions the way the server parses them", () => {
    const params = toQueryParams(filters({ status: ["running", "failed"], source: ["slack"] }));
    expect(params.status).toBe("running,failed");
    expect(params.source).toBe("slack");
  });

  it("quantizes the window bound so the query key is stable between renders", () => {
    const base = Date.parse("2026-08-09T12:34:56.789Z");
    expect(windowStart("24h", base)).toBe(windowStart("24h", base + 1_000));
    expect(windowStart("24h", base)).toBe("2026-08-08T12:34:00.000Z");
    expect(windowStart("all", base)).toBeUndefined();
  });
});

describe("active filter detection", () => {
  it("ignores pagination", () => {
    expect(hasActiveFilters(filters({ page: 3 }))).toBe(false);
  });

  it("notices every dimension", () => {
    expect(hasActiveFilters(filters({ mine: true }))).toBe(true);
    expect(hasActiveFilters(filters({ window: "24h" }))).toBe(true);
    expect(hasActiveFilters(filters({ agent: "agent_1" }))).toBe(true);
  });
});

describe("saved views", () => {
  it("drops malformed stored entries rather than rendering junk", () => {
    expect(
      parseSavedViews([
        { id: "a", name: "Failures", query: "status=failed" },
        { id: "b", name: 7 },
        "nope",
        null,
      ]),
    ).toEqual([{ id: "a", name: "Failures", query: "status=failed" }]);
  });

  it("treats a view as active regardless of the page being read", () => {
    const view = { id: "a", name: "Failures", query: "status=failed&window=7d" };
    expect(isViewActive(view, filters({ status: ["failed"], window: "7d", page: 2 }))).toBe(true);
    expect(isViewActive(view, filters({ status: ["failed"] }))).toBe(false);
  });

  it("matches views whose stored query is not in canonical form", () => {
    const view = { id: "a", name: "Failures", query: "?page=0&status=failed&mine=false" };
    expect(isViewActive(view, filters({ status: ["failed"] }))).toBe(true);
  });
});
