import {
  accountLabel,
  attemptStatus,
  automaticEntries,
  automaticOrder,
  describePlan,
  moveAutomaticEntry,
  parseWebSearchDetails,
  settingLabel,
  summarizeWebSearch,
} from "@/lib/web-search";
import type { SearchAccountSummary, SearchPlanPreview } from "@/lib/api/types";
import { BRAVE_ACCOUNT_ID, fallbackDetails } from "@/app/dev/_fixtures/web-search-fixtures";

const BRAVE = BRAVE_ACCOUNT_ID;
const PARALLEL = "00000000-0000-0000-0000-000000000008";

function account(id: string, name: string, status = "ready"): SearchAccountSummary {
  return {
    id,
    name,
    provider: "brave_search",
    status,
    automatic_position: null,
    connected_at: "2026-10-11T00:00:00Z",
  };
}

describe("accountLabel", () => {
  it("names the connector, then the account", () => {
    expect(accountLabel({ provider: "cloudflare_web_search", name: "Team" })).toBe(
      "Cloudflare Web Search · Team",
    );
    expect(accountLabel({ provider: "parallel", name: "parallel" })).toBe("Parallel");
  });
});

describe("parseWebSearchDetails", () => {
  it("reads web search details and ignores anything else", () => {
    expect(parseWebSearchDetails(fallbackDetails)?.attempts).toHaveLength(3);
    expect(parseWebSearchDetails({ kind: "other", query: "x" })).toBeNull();
    expect(parseWebSearchDetails(null)).toBeNull();
    expect(parseWebSearchDetails("web_search")).toBeNull();
  });
});

describe("summarizeWebSearch", () => {
  it("names who answered and why an earlier provider did not", () => {
    const summary = summarizeWebSearch(parseWebSearchDetails(fallbackDetails)!, 1300);
    expect(summary).toMatchObject({
      query: "tokio-console 0.2 release",
      resultCount: 2,
      providerLabel: "Free search",
      durationMs: 1300,
      // The skipped native step is not a fallback worth a note.
      fallbackNote: "Brave Search was rate limited",
      failed: false,
    });
  });

  it("reports a search nobody answered", () => {
    const summary = summarizeWebSearch(
      parseWebSearchDetails({
        ...fallbackDetails,
        ok: false,
        results: [],
        provider: undefined,
        provider_label: undefined,
        attempts: [fallbackDetails.attempts[1]],
      })!,
    );
    expect(summary.failed).toBe(true);
    expect(summary.providerLabel).toBeNull();
    expect(summary.failureReason).toBe("Brave Search returned 429");
    expect(summary.durationMs).toBe(200);
  });
});

describe("attemptStatus and settingLabel", () => {
  it("describes each attempt the way the details view lists them", () => {
    const [native, brave, free] = parseWebSearchDetails(fallbackDetails)!.attempts;
    expect(attemptStatus(native)).toBe("skipped · This model has no search of its own");
    expect(attemptStatus(brave)).toBe("429 rate limited");
    expect(attemptStatus(free)).toBe("2 results");
  });

  it("shows what the organization default resolved to", () => {
    expect(settingLabel(parseWebSearchDetails(fallbackDetails)!)).toBe(
      "Organization default · Automatic",
    );
    const own = parseWebSearchDetails({
      ...fallbackDetails,
      setting: { provider: "account", account_id: BRAVE },
    })!;
    expect(settingLabel(own, [{ id: BRAVE, name: "Brave Search · Team" }])).toBe(
      "Brave Search · Team",
    );
  });
});

describe("Automatic order", () => {
  it("keeps unlisted entries in their natural place and free search last", () => {
    const entries = automaticEntries({ automatic_order: [PARALLEL, "free"] }, [
      account(BRAVE, "Team"),
      account(PARALLEL, "Backup"),
    ]);
    expect(entries.map((entry) => entry.id)).toEqual(["native", PARALLEL, BRAVE, "free"]);
  });

  it("follows a stored order that puts an account above native search", () => {
    const entries = automaticEntries({ automatic_order: [BRAVE, "native", "free"] }, [
      account(BRAVE, "Team"),
    ]);
    expect(entries.map((entry) => entry.id)).toEqual([BRAVE, "native", "free"]);
  });

  it("marks native search off and accounts without a key as inactive", () => {
    const entries = automaticEntries({ allow_native: false }, [
      account(BRAVE, "Team", "missing_key"),
    ]);
    expect(entries.map((entry) => [entry.label, entry.inactive])).toEqual([
      ["Model provider search", true],
      ["Brave Search · Team", true],
      ["Free search", false],
    ]);
  });

  it("drops accounts that no longer exist", () => {
    const entries = automaticEntries({ automatic_order: [PARALLEL, "native", "free"] }, []);
    expect(entries.map((entry) => entry.id)).toEqual(["native", "free"]);
  });

  it("moves entries but never past free search", () => {
    const entries = automaticEntries({}, [account(BRAVE, "Team")]);
    const up = moveAutomaticEntry(entries, 1, -1);
    expect(automaticOrder(up)).toEqual([BRAVE, "native", "free"]);
    expect(moveAutomaticEntry(entries, 1, 1)).toBe(entries);
    expect(moveAutomaticEntry(entries, 2, -1)).toBe(entries);
  });
});

describe("describePlan", () => {
  const preview: SearchPlanPreview = {
    provider: "auto",
    fallback: "next",
    limit: 8,
    model_has_native: true,
    org_blocked_domains: [],
    steps: [
      {
        id: "native",
        label: "Model provider search",
        available: true,
        price_note: "Billed by the model provider, about $10 per 1,000 searches",
      },
      { id: `account:${BRAVE}`, label: "Brave Search · Team", available: true },
      { id: "free", label: "Free search", available: true },
    ],
  };

  it("says who answers first, what it costs and what comes next", () => {
    expect(describePlan(preview, "Claude Sonnet 5.5")).toEqual([
      "With Claude Sonnet 5.5: Model provider search (billed by the model provider, about $10 per 1,000 searches) answers first.",
      "When it cannot answer: Brave Search · Team, then Free search.",
      "8 results per search unless the agent asks for fewer.",
    ]);
  });

  it("skips providers that cannot answer and says why native is skipped", () => {
    const lines = describePlan({
      ...preview,
      model_has_native: false,
      steps: [
        { ...preview.steps[0], available: false, note: "This model has no search of its own" },
        preview.steps[2],
      ],
    });
    expect(lines[0]).toBe("With the default model: Free search answers first.");
    expect(lines).toContain(
      "Model provider search is skipped: this model has no search of its own.",
    );
  });

  it("says when fallback is off", () => {
    const lines = describePlan({ ...preview, fallback: "none" });
    expect(lines[1]).toBe("When it cannot answer, the search fails: fallback is off.");
  });
});
