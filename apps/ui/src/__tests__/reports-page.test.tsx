import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import ReportsPage from "@/app/(main)/reports/page";
import type { OrgRole, ReportQuery, ReportResult, SavedReport } from "@/lib/api/types";

const mockUseReportCatalog = jest.fn();
const mockUseSavedReports = jest.fn();
const mockUseReportQuery = jest.fn();
const mockUseReportingDiagnostics = jest.fn();
const mockCreateSavedReport = jest.fn();
const mockDeleteSavedReport = jest.fn();
const mockRunSavedReport = jest.fn();
const mockExportReportQuery = jest.fn();
const mockExportSavedReport = jest.fn();
const mockRunReportingProjector = jest.fn();
const mockBackfillReporting = jest.fn();

let currentRole: OrgRole = "member";

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({
    hasRole: (role: OrgRole) => {
      const hierarchy: Record<OrgRole, number> = { owner: 3, admin: 2, member: 1 };
      return hierarchy[currentRole] >= hierarchy[role];
    },
  }),
}));

jest.mock("@/hooks", () => ({
  usePageTitle: () => undefined,
  useReportCatalog: () => mockUseReportCatalog(),
  useSavedReports: () => mockUseSavedReports(),
  useReportQuery: (query: ReportQuery | undefined) => mockUseReportQuery(query),
  useReportingDiagnostics: (enabled?: boolean) => mockUseReportingDiagnostics(enabled),
  useCreateSavedReport: () => mockCreateSavedReport(),
  useDeleteSavedReport: () => mockDeleteSavedReport(),
  useRunSavedReport: () => mockRunSavedReport(),
  useExportReportQuery: () => mockExportReportQuery(),
  useExportSavedReport: () => mockExportSavedReport(),
  useRunReportingProjector: () => mockRunReportingProjector(),
  useBackfillReporting: () => mockBackfillReporting(),
}));

const savedQuery: ReportQuery = {
  dataset: "tool_calls",
  time_range: {
    from: "2026-05-16T00:00:00.000Z",
    to: "2026-05-17T00:00:00.000Z",
  },
  dimensions: ["tool", "agent"],
  measures: ["call_count"],
  filters: [{ field: "tool", op: "eq", value: "web_fetch" }],
  order_by: [{ measure: "call_count", direction: "desc" }],
  limit: 25,
};

const savedReport: SavedReport = {
  id: "report_1",
  name: "Filtered Tool Calls",
  query: savedQuery,
  dashboard: { chart_type: "table" },
  created_at: "2026-05-17T00:00:00.000Z",
  updated_at: "2026-05-17T00:00:00.000Z",
};

const emptyResult: ReportResult = {
  as_of: "2026-05-17T00:00:00.000Z",
  freshness_lag_ms: null,
  columns: [],
  rows: [],
};

function renderReportsPage(role: OrgRole = "member", reports: SavedReport[] = []) {
  currentRole = role;
  mockUseReportCatalog.mockReturnValue({
    data: {
      datasets: [
        {
          name: "tool_calls",
          dimensions: ["day", "tool", "agent"],
          measures: ["call_count"],
          filter_fields: ["tool"],
        },
      ],
    },
    isLoading: false,
  });
  mockUseSavedReports.mockReturnValue({ data: reports });
  mockUseReportQuery.mockReturnValue({ data: emptyResult, isFetching: false, error: null });
  mockUseReportingDiagnostics.mockReturnValue({
    data: {
      projector_lag: [{ dataset: "tool_calls", latest_projected_at: null, freshness_lag_ms: null }],
      outbox: {
        pending: 0,
        processing: 0,
        failed: 0,
        completed: 0,
        oldest_pending_at: null,
        failed_rows: [],
      },
    },
  });
  mockCreateSavedReport.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
  mockDeleteSavedReport.mockReturnValue({ mutate: jest.fn(), isPending: false });
  mockRunSavedReport.mockReturnValue({ mutateAsync: jest.fn().mockResolvedValue(emptyResult) });
  mockExportReportQuery.mockReturnValue({
    mutateAsync: jest.fn().mockResolvedValue({
      format: "csv",
      filename: "report.csv",
      content_type: "text/csv; charset=utf-8",
      content: "",
      as_of: "2026-05-17T00:00:00.000Z",
    }),
    isPending: false,
  });
  mockExportSavedReport.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
  mockRunReportingProjector.mockReturnValue({ mutate: jest.fn(), isPending: false });
  mockBackfillReporting.mockReturnValue({ mutate: jest.fn(), isPending: false });

  render(<ReportsPage />);
}

describe("ReportsPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    URL.createObjectURL = jest.fn(() => "blob:report");
    URL.revokeObjectURL = jest.fn();
    HTMLAnchorElement.prototype.click = jest.fn();
  });

  it("renders empty report states for report viewers without admin requests", () => {
    renderReportsPage("member");

    expect(screen.getByText("No saved reports.")).toBeInTheDocument();
    expect(screen.getByText("No rows for this query.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save report" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Backfill" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Project" })).not.toBeInTheDocument();
    expect(mockUseReportingDiagnostics).toHaveBeenCalledWith(false);
  });

  it("allows org admins to manage saved reports but not run admin projection actions", () => {
    renderReportsPage("admin", [savedReport]);

    expect(screen.getByRole("button", { name: "Save report" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete saved report" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Backfill" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Project" })).not.toBeInTheDocument();
    expect(screen.queryByText("Projection Lag")).not.toBeInTheDocument();
    expect(mockUseReportingDiagnostics).toHaveBeenCalledWith(false);
  });

  it("allows org owners to run reporting admin actions", () => {
    renderReportsPage("owner", [savedReport]);

    expect(screen.getByRole("button", { name: "Backfill" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Project" })).toBeInTheDocument();
    expect(screen.getByText("Projection Lag")).toBeInTheDocument();
    expect(mockUseReportingDiagnostics).toHaveBeenCalledWith(true);
  });

  it("exports the full saved report query after running a saved report", async () => {
    renderReportsPage("admin", [savedReport]);

    fireEvent.click(screen.getByText("Filtered Tool Calls"));
    await waitFor(() => expect(mockRunSavedReport().mutateAsync).toHaveBeenCalledWith("report_1"));

    fireEvent.click(screen.getByRole("button", { name: "CSV" }));

    await waitFor(() => {
      expect(mockExportReportQuery().mutateAsync).toHaveBeenCalledWith({
        query: savedQuery,
        format: "csv",
      });
    });
  });
});
