import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import WorkersPage from "@/app/(main)/durable/workers/page";
import type { DurableWorker, WorkersResponse, WorkersSummary } from "@/lib/api/types";

// Mock worker data
function makeWorker(overrides: Partial<DurableWorker> = {}): DurableWorker {
  return {
    id: "worker-abc-123",
    worker_group: "default",
    activity_types: ["run_session"],
    max_concurrency: 10,
    current_load: 3,
    status: "active",
    accepting_tasks: true,
    started_at: "2024-01-01T00:00:00Z",
    last_heartbeat_at: new Date().toISOString(),
    hostname: "host-1",
    version: "0.8.1",
    tasks_completed: 100,
    tasks_failed: 2,
    avg_task_duration_ms: 500,
    ...overrides,
  };
}

const mockActiveWorker = makeWorker({
  id: "w-active-1",
  status: "active",
  max_concurrency: 10,
  current_load: 3,
});
const mockDrainingWorker = makeWorker({
  id: "w-draining-1",
  status: "draining",
  max_concurrency: 8,
  current_load: 2,
  accepting_tasks: false,
});
const mockStoppedWorker = makeWorker({
  id: "w-stopped-1",
  status: "stopped",
  max_concurrency: 10,
  current_load: 0,
  accepting_tasks: false,
});

const mockSummary: WorkersSummary = {
  active: 1,
  draining: 1,
  stopped: 1,
  total_capacity: 10,
  total_load: 3,
};

const mockWorkersResponse: WorkersResponse = {
  data: [mockActiveWorker, mockDrainingWorker, mockStoppedWorker],
  total: 3,
  summary: mockSummary,
};

// Mock hooks
const mockUseWorkers = jest.fn();
const mockUseDrainWorker = jest.fn();
const mockUseResumeWorker = jest.fn();

jest.mock("@/hooks", () => ({
  useWorkers: () => mockUseWorkers(),
  useDrainWorker: () => mockUseDrainWorker(),
  useResumeWorker: () => mockUseResumeWorker(),
}));

// Mock Next.js navigation
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: jest.fn() }),
  usePathname: () => "/durable/workers",
}));

// Mock window.confirm
const mockConfirm = jest.fn();
window.confirm = mockConfirm;

describe("WorkersPage", () => {
  let queryClient: QueryClient;

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });

    jest.clearAllMocks();

    // Default mock return values
    mockUseWorkers.mockReturnValue({
      data: mockWorkersResponse,
      isLoading: false,
      error: null,
      refetch: jest.fn(),
    });

    mockUseDrainWorker.mockReturnValue({
      mutate: jest.fn(),
      isPending: false,
    });

    mockUseResumeWorker.mockReturnValue({
      mutate: jest.fn(),
      isPending: false,
    });

    mockConfirm.mockReturnValue(true);
  });

  // --------------------------------------------
  // Loading State Tests
  // --------------------------------------------

  describe("Loading State", () => {
    it("shows skeleton loading state", () => {
      mockUseWorkers.mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
        refetch: jest.fn(),
      });

      render(<WorkersPage />, { wrapper });

      const skeletons = document.querySelectorAll('[class*="animate-pulse"]');
      expect(skeletons.length).toBeGreaterThan(0);
    });

    it("renders header during loading", () => {
      mockUseWorkers.mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
        refetch: jest.fn(),
      });

      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("Workers")).toBeInTheDocument();
    });
  });

  // --------------------------------------------
  // Error State Tests
  // --------------------------------------------

  describe("Error State", () => {
    it("shows error message when workers fail to load", () => {
      mockUseWorkers.mockReturnValue({
        data: undefined,
        isLoading: false,
        error: new Error("Network error"),
        refetch: jest.fn(),
      });

      render(<WorkersPage />, { wrapper });

      expect(screen.getByText(/Unable to Load Workers/i)).toBeInTheDocument();
    });

    it("shows retry button on error", () => {
      mockUseWorkers.mockReturnValue({
        data: undefined,
        isLoading: false,
        error: new Error("Network error"),
        refetch: jest.fn(),
      });

      render(<WorkersPage />, { wrapper });

      expect(screen.getByRole("button", { name: /Retry/i })).toBeInTheDocument();
    });

    it("calls refetch when retry button is clicked", () => {
      const mockRefetch = jest.fn();
      mockUseWorkers.mockReturnValue({
        data: undefined,
        isLoading: false,
        error: new Error("Network error"),
        refetch: mockRefetch,
      });

      render(<WorkersPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Retry/i }));
      expect(mockRefetch).toHaveBeenCalled();
    });
  });

  // --------------------------------------------
  // Empty State Tests
  // --------------------------------------------

  describe("Empty State", () => {
    it("shows empty state when no workers exist", () => {
      mockUseWorkers.mockReturnValue({
        data: {
          data: [],
          total: 0,
          summary: { active: 0, draining: 0, stopped: 0, total_capacity: 0, total_load: 0 },
        },
        isLoading: false,
        error: null,
        refetch: jest.fn(),
      });

      render(<WorkersPage />, { wrapper });

      expect(screen.getByText(/No Workers Connected/i)).toBeInTheDocument();
    });
  });

  // --------------------------------------------
  // Summary Stats Card Tests
  // --------------------------------------------

  describe("Summary Stats Cards", () => {
    it("displays total workers count", () => {
      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("Total Workers")).toBeInTheDocument();
      // total = 3 (from mockWorkersResponse.total)
      expect(screen.getByText("3")).toBeInTheDocument();
    });

    it("displays active workers count in description", () => {
      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("1 active")).toBeInTheDocument();
    });

    it("displays total capacity from summary", () => {
      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("Total Capacity")).toBeInTheDocument();
      expect(screen.getByText("10")).toBeInTheDocument();
    });

    it("displays total load in capacity description", () => {
      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("3 in use")).toBeInTheDocument();
    });

    it("displays load percentage", () => {
      render(<WorkersPage />, { wrapper });

      // "Load" appears as both a card title and table header; verify via card title selector
      const loadCards = screen.getAllByText("Load");
      expect(loadCards.length).toBeGreaterThanOrEqual(1);
      // 3/10 * 100 = 30.0%
      expect(screen.getByText("30.0%")).toBeInTheDocument();
    });

    it("displays load slots description", () => {
      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("3/10 slots")).toBeInTheDocument();
    });

    it("displays draining count", () => {
      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("Draining")).toBeInTheDocument();
    });

    it("displays stopped count in draining description", () => {
      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("1 stopped")).toBeInTheDocument();
    });

    it("shows 0% load when no capacity", () => {
      mockUseWorkers.mockReturnValue({
        data: {
          data: [],
          total: 0,
          summary: { active: 0, draining: 0, stopped: 0, total_capacity: 0, total_load: 0 },
        },
        isLoading: false,
        error: null,
        refetch: jest.fn(),
      });

      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("0%")).toBeInTheDocument();
    });

    it("handles missing summary gracefully", () => {
      mockUseWorkers.mockReturnValue({
        data: { data: [], total: 0 },
        isLoading: false,
        error: null,
        refetch: jest.fn(),
      });

      render(<WorkersPage />, { wrapper });

      // Should still render without crashing, using fallback values
      expect(screen.getByText("Total Workers")).toBeInTheDocument();
      expect(screen.getByText("0 active")).toBeInTheDocument();
    });
  });

  // --------------------------------------------
  // Worker Table Rendering Tests
  // --------------------------------------------

  describe("Worker Table Rendering", () => {
    it("renders worker table with correct headers", () => {
      render(<WorkersPage />, { wrapper });

      // Some header names overlap with card titles (e.g. "Load"), so use getAllByText
      expect(screen.getByText("Worker")).toBeInTheDocument();
      expect(screen.getByText("Status")).toBeInTheDocument();
      expect(screen.getByText("Group")).toBeInTheDocument();
      expect(screen.getAllByText("Load").length).toBeGreaterThanOrEqual(1);
      expect(screen.getByText("Accepting")).toBeInTheDocument();
      expect(screen.getByText("Activities")).toBeInTheDocument();
      expect(screen.getByText("Completed")).toBeInTheDocument();
      expect(screen.getByText("Avg Duration")).toBeInTheDocument();
      expect(screen.getByText("Last Heartbeat")).toBeInTheDocument();
      expect(screen.getByText("Actions")).toBeInTheDocument();
    });

    it("renders worker hostnames", () => {
      render(<WorkersPage />, { wrapper });

      // All 3 workers share hostname "host-1"
      const hostnames = screen.getAllByText("host-1");
      expect(hostnames.length).toBe(3);
    });

    it("renders status badges for each worker", () => {
      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("active")).toBeInTheDocument();
      expect(screen.getByText("draining")).toBeInTheDocument();
      expect(screen.getByText("stopped")).toBeInTheDocument();
    });

    it("renders worker group badges", () => {
      render(<WorkersPage />, { wrapper });

      // All workers have "default" group
      const defaultBadges = screen.getAllByText("default");
      expect(defaultBadges.length).toBe(3);
    });

    it("renders activity type badges", () => {
      render(<WorkersPage />, { wrapper });

      const activityBadges = screen.getAllByText("run_session");
      expect(activityBadges.length).toBe(3);
    });

    it("renders task completed counts", () => {
      render(<WorkersPage />, { wrapper });

      // All workers have 100 tasks_completed
      const completedCounts = screen.getAllByText("100");
      expect(completedCounts.length).toBe(3);
    });
  });

  // --------------------------------------------
  // Worker Action Tests
  // --------------------------------------------

  describe("Worker Actions", () => {
    it("shows Drain button for active workers", () => {
      render(<WorkersPage />, { wrapper });

      const drainButton = screen.getByRole("button", { name: /Drain/i });
      expect(drainButton).toBeInTheDocument();
    });

    it("shows Resume button for draining workers", () => {
      render(<WorkersPage />, { wrapper });

      const resumeButton = screen.getByRole("button", { name: /Resume/i });
      expect(resumeButton).toBeInTheDocument();
    });

    it("calls drain mutation after confirmation", () => {
      const mockDrainMutate = jest.fn();
      mockUseDrainWorker.mockReturnValue({
        mutate: mockDrainMutate,
        isPending: false,
      });
      mockConfirm.mockReturnValue(true);

      render(<WorkersPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Drain/i }));
      expect(mockConfirm).toHaveBeenCalled();
      expect(mockDrainMutate).toHaveBeenCalledWith("w-active-1");
    });

    it("does not drain when confirmation is cancelled", () => {
      const mockDrainMutate = jest.fn();
      mockUseDrainWorker.mockReturnValue({
        mutate: mockDrainMutate,
        isPending: false,
      });
      mockConfirm.mockReturnValue(false);

      render(<WorkersPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Drain/i }));
      expect(mockConfirm).toHaveBeenCalled();
      expect(mockDrainMutate).not.toHaveBeenCalled();
    });

    it("calls resume mutation without confirmation", () => {
      const mockResumeMutate = jest.fn();
      mockUseResumeWorker.mockReturnValue({
        mutate: mockResumeMutate,
        isPending: false,
      });

      render(<WorkersPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Resume/i }));
      expect(mockResumeMutate).toHaveBeenCalledWith("w-draining-1");
    });

    it("does not show action buttons for stopped workers", () => {
      mockUseWorkers.mockReturnValue({
        data: {
          data: [mockStoppedWorker],
          total: 1,
          summary: { active: 0, draining: 0, stopped: 1, total_capacity: 0, total_load: 0 },
        },
        isLoading: false,
        error: null,
        refetch: jest.fn(),
      });

      render(<WorkersPage />, { wrapper });

      expect(screen.queryByRole("button", { name: /Drain/i })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /Resume/i })).not.toBeInTheDocument();
    });
  });

  // --------------------------------------------
  // Summary with various worker configurations
  // --------------------------------------------

  describe("Summary with different worker configurations", () => {
    it("handles all draining workers", () => {
      const allDraining: WorkersResponse = {
        data: [
          makeWorker({
            id: "w1",
            status: "draining",
            max_concurrency: 10,
            current_load: 5,
            accepting_tasks: false,
          }),
          makeWorker({
            id: "w2",
            status: "draining",
            max_concurrency: 10,
            current_load: 3,
            accepting_tasks: false,
          }),
        ],
        total: 2,
        summary: { active: 0, draining: 2, stopped: 0, total_capacity: 0, total_load: 0 },
      };

      mockUseWorkers.mockReturnValue({
        data: allDraining,
        isLoading: false,
        error: null,
        refetch: jest.fn(),
      });

      render(<WorkersPage />, { wrapper });

      expect(screen.getByText("0 active")).toBeInTheDocument();
      expect(screen.getByText("0%")).toBeInTheDocument();
    });

    it("handles high load scenario", () => {
      const highLoad: WorkersResponse = {
        data: [makeWorker({ id: "w1", status: "active", max_concurrency: 10, current_load: 9 })],
        total: 1,
        summary: { active: 1, draining: 0, stopped: 0, total_capacity: 10, total_load: 9 },
      };

      mockUseWorkers.mockReturnValue({
        data: highLoad,
        isLoading: false,
        error: null,
        refetch: jest.fn(),
      });

      render(<WorkersPage />, { wrapper });

      // 9/10 * 100 = 90.0%
      expect(screen.getByText("90.0%")).toBeInTheDocument();
      expect(screen.getByText("9/10 slots")).toBeInTheDocument();
    });
  });
});
