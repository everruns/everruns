import { fireEvent, render, screen, within } from "@testing-library/react";
import type { ReactElement } from "react";
import { ToolActivityTimelineGroup } from "@/components/chat/tool-activity-timeline-group";
import type { ToolCompletedData } from "@/lib/api/types";
import { LocaleProvider } from "@/providers/locale-provider";
import { fallbackDetails } from "@/app/dev/_fixtures/web-search-fixtures";

function renderWithLocale(ui: ReactElement) {
  return render(<LocaleProvider>{ui}</LocaleProvider>);
}

function searchResult(details: unknown): ToolCompletedData {
  return {
    tool_call_id: "call-1",
    tool_name: "web_search",
    success: true,
    status: "success",
    duration_ms: 1300,
    narration: "Found 2 results with Free search (Brave Search was rate limited)",
    result: [{ type: "text", text: "{}" }],
    details,
  };
}

function renderSearch(details: unknown) {
  return renderWithLocale(
    <ToolActivityTimelineGroup
      headline="Found 2 results"
      rows={[
        {
          id: "call-1",
          label: "Searching the web",
          state: "completed",
          arguments: { query: fallbackDetails.query },
          result: searchResult(details),
        },
      ]}
    />,
  );
}

describe("web search chat row", () => {
  it("says what was searched, who answered and why a fallback happened", () => {
    renderSearch(fallbackDetails);
    const row = screen.getByTestId("web-search-row");
    expect(within(row).getByText("Searched")).toBeInTheDocument();
    expect(within(row).getByText('"tokio-console 0.2 release"')).toBeInTheDocument();
    expect(within(row).getByText("2 results")).toBeInTheDocument();
    expect(
      within(row).getByText("Free search · Brave Search was rate limited"),
    ).toBeInTheDocument();
    // Results stay folded until asked for.
    expect(screen.queryByText("Announcing tokio-console 0.2")).not.toBeInTheDocument();
  });

  it("lists results when expanded and opens the details and raw views", () => {
    renderSearch(fallbackDetails);
    fireEvent.click(screen.getByRole("button", { name: "Show results" }));
    expect(screen.getByText("Announcing tokio-console 0.2")).toBeInTheDocument();
    expect(screen.getByText("tokio.rs")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Details and raw response" }));
    const attempts = screen.getByRole("list", { name: "Attempts" });
    expect(within(attempts).getByText("429 rate limited · 200ms")).toBeInTheDocument();
    expect(within(attempts).getByText(/skipped · This model has no search/)).toBeInTheDocument();
    expect(screen.getByText("Organization default · Automatic")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Raw" }));
    // The provider that answered is selected first.
    expect(screen.getByText(/GET https:\/\/html\.duckduckgo\.com/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "2 · Brave Search" }));
    expect(screen.getByText(/api\.search\.brave\.com/)).toBeInTheDocument();
  });

  it("shows a failed search with its reason", () => {
    renderSearch({
      ...fallbackDetails,
      ok: false,
      results: [],
      provider: undefined,
      provider_label: undefined,
      attempts: [fallbackDetails.attempts[1]],
    });
    expect(screen.getByText("Search failed")).toBeInTheDocument();
    expect(screen.getByText("Brave Search returned 429")).toBeInTheDocument();
  });

  it("keeps the generic row for other tools", () => {
    renderSearch({ kind: "something_else" });
    expect(screen.queryByTestId("web-search-row")).not.toBeInTheDocument();
    expect(screen.getByText("Found 2 results")).toBeInTheDocument();
  });
});
