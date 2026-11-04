import { fireEvent, render, screen } from "@testing-library/react";
import SystemHealthPage from "@/app/(main)/system/health/page";
import type { DoctorReport } from "@/lib/api/types";
import { renderHook } from "@testing-library/react";
import {
  PageViewRegistryContext,
  usePageViewRegistry,
} from "@/components/chat-mode/page-view-state";

const mockRefresh = jest.fn();
let mockReport: DoctorReport | undefined;
let mockPlatformAccess = false;
let mockPolicyLoading = false;
let mockPolicyError = false;
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({
    can: () => mockPlatformAccess,
    isLoading: mockPolicyLoading,
    error: mockPolicyError ? new Error("policy unavailable") : null,
  }),
}));

jest.mock("@/hooks", () => ({
  usePageTitle: () => {},
  useSystemDoctor: () => ({ data: mockReport, isLoading: false, error: null }),
  useRefreshSystemDoctor: () => ({ mutate: mockRefresh, isPending: false }),
}));

const report: DoctorReport = {
  status: "problem",
  checked_at: new Date().toISOString(),
  version: "0.50.0",
  grade: "prod",
  auth_mode: "full",
  setup: ["PostgreSQL 17.2", "2 API servers"],
  checks: [
    {
      id: "database.migrations",
      area: "database",
      title: "Migrations applied",
      status: "problem",
      summary: "2 migrations are missing.",
      why: "New features fail on the old schema.",
      fix: "Run everruns-migrate.",
      observed_at: new Date().toISOString(),
    },
    {
      id: "database.reachable",
      area: "database",
      title: "Database reachable",
      status: "ok",
      summary: "The database answered in 3 ms.",
      observed_at: new Date().toISOString(),
    },
    {
      id: "messaging.nats",
      area: "messaging",
      title: "NATS",
      status: "info",
      summary: "Not used.",
      observed_at: new Date().toISOString(),
    },
  ],
};

describe("System → Health", () => {
  beforeEach(() => {
    mockReport = report;
    mockPlatformAccess = false;
    mockPolicyLoading = false;
    mockPolicyError = false;
    mockRefresh.mockClear();
  });

  it("shows the verdict, the fix for each problem and the rest by area", () => {
    render(<SystemHealthPage />);
    expect(screen.getByText("Needs attention: 1 problem.")).toBeInTheDocument();
    expect(screen.getByText("PostgreSQL 17.2 · 2 API servers")).toBeInTheDocument();
    expect(screen.getByText("Needs attention")).toBeInTheDocument();
    expect(screen.getByText("Run everruns-migrate.")).toBeInTheDocument();
    expect(screen.getByText("The database answered in 3 ms.")).toBeInTheDocument();
    expect(screen.getByText("Messaging")).toBeInTheDocument();
  });

  it("runs the checks again on request", () => {
    render(<SystemHealthPage />);
    fireEvent.click(screen.getByRole("button", { name: /check again/i }));
    expect(mockRefresh).toHaveBeenCalledTimes(1);
  });

  it("explains when the report cannot be loaded", () => {
    mockReport = undefined;
    render(<SystemHealthPage />);
    expect(screen.getByText("Could not load the health report")).toBeInTheDocument();
  });

  it.each(["allowed", "denied", "loading", "error"])(
    "projects only bounded health status with platform policy %s",
    (state) => {
      const allowed = state === "allowed";
      mockPlatformAccess = state !== "denied";
      mockPolicyLoading = state === "loading";
      mockPolicyError = state === "error";
      const registry = renderHook(() => usePageViewRegistry("/system/health"));
      render(
        <PageViewRegistryContext.Provider value={registry.result.current.registry}>
          <SystemHealthPage />
        </PageViewRegistryContext.Provider>,
      );
      const data = registry.result.current.read("/system/health");
      expect(data.sections).toHaveLength(2);
      if (allowed)
        expect(data.sections[1]).toMatchObject({
          rows: [
            {
              id: "database.migrations",
              area: "database",
              title: "Migrations applied",
              status: "problem",
            },
            {
              id: "database.reachable",
              area: "database",
              title: "Database reachable",
              status: "ok",
            },
            { id: "messaging.nats", area: "messaging", title: "NATS", status: "info" },
          ],
        });
      else
        data.sections.forEach((s) =>
          expect(s).toMatchObject({ rows: [], load_state: "unavailable" }),
        );
      expect(JSON.stringify(data)).not.toContain("3 ms");
      expect(JSON.stringify(data)).not.toContain("Run everruns-migrate");
    },
  );
});
