import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import ScheduleDetailPage from "@/app/(main)/durable/schedules/[scheduleId]/page";
import type {
  DurableSchedule,
  ScheduleExecution,
  ScheduleStats,
  ScheduleExecutionsResponse,
} from "@/lib/api/types";

// Mock schedule data
const mockSchedule: DurableSchedule = {
  id: "sched_123",
  name: "Daily Backup",
  description: "Runs daily backup workflow for all services",
  cron_expression: "0 0 0 * * * *",
  timezone: "UTC",
  target: {
    type: "workflow",
    name: "backup-workflow",
    input: { bucket: "backups", retention: 30 },
  },
  enabled: true,
  max_concurrent: 2,
  catch_up_missed: false,
  max_catch_up: 10,
  last_triggered_at: "2024-01-15T00:00:00Z",
  next_trigger_at: "2024-01-16T00:00:00Z",
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
};

const mockExecutions: ScheduleExecution[] = [
  {
    id: "exec_1",
    schedule_id: "sched_123",
    scheduled_at: "2024-01-15T00:00:00Z",
    started_at: "2024-01-15T00:00:01Z",
    completed_at: "2024-01-15T00:00:05Z",
    status: "completed",
    workflow_id: "wf_456",
    duration_ms: 4000,
    created_at: "2024-01-15T00:00:00Z",
  },
  {
    id: "exec_2",
    schedule_id: "sched_123",
    scheduled_at: "2024-01-14T00:00:00Z",
    started_at: "2024-01-14T00:00:01Z",
    completed_at: "2024-01-14T00:00:10Z",
    status: "failed",
    workflow_id: "wf_457",
    duration_ms: 9000,
    error: "Connection timeout",
    created_at: "2024-01-14T00:00:00Z",
  },
  {
    id: "exec_3",
    schedule_id: "sched_123",
    scheduled_at: "2024-01-13T00:00:00Z",
    started_at: "2024-01-13T00:00:01Z",
    status: "running",
    created_at: "2024-01-13T00:00:00Z",
  },
  {
    id: "exec_4",
    schedule_id: "sched_123",
    scheduled_at: "2024-01-12T00:00:00Z",
    started_at: "2024-01-12T00:00:01Z",
    completed_at: "2024-01-12T00:00:02Z",
    status: "skipped",
    duration_ms: 1000,
    created_at: "2024-01-12T00:00:00Z",
  },
];

const mockStats: ScheduleStats = {
  total_executions: 100,
  successful_executions: 95,
  failed_executions: 3,
  skipped_executions: 2,
  avg_duration_ms: 3500,
  last_execution_status: "completed",
};

// Mock hooks
const mockUseSchedule = jest.fn();
const mockUseScheduleExecutions = jest.fn();
const mockUseScheduleStats = jest.fn();
const mockUseUpdateSchedule = jest.fn();
const mockUsePauseSchedule = jest.fn();
const mockUseResumeSchedule = jest.fn();
const mockUseTriggerSchedule = jest.fn();
const mockUseDeleteSchedule = jest.fn();

jest.mock("@/hooks/use-durable", () => ({
  useSchedule: () => mockUseSchedule(),
  useScheduleExecutions: () => mockUseScheduleExecutions(),
  useScheduleStats: () => mockUseScheduleStats(),
  useUpdateSchedule: () => mockUseUpdateSchedule(),
  usePauseSchedule: () => mockUsePauseSchedule(),
  useResumeSchedule: () => mockUseResumeSchedule(),
  useTriggerSchedule: () => mockUseTriggerSchedule(),
  useDeleteSchedule: () => mockUseDeleteSchedule(),
}));

// Mock Next.js navigation
const mockPush = jest.fn();
const mockParams = { scheduleId: "sched_123" };

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush }),
  useParams: () => mockParams,
  usePathname: () => "/durable/schedules/sched_123",
}));

// Mock window.confirm for delete/trigger actions
const mockConfirm = jest.fn();
window.confirm = mockConfirm;

describe("ScheduleDetailPage", () => {
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
    mockUseSchedule.mockReturnValue({
      data: mockSchedule,
      isLoading: false,
      error: null,
      refetch: jest.fn(),
    });

    mockUseScheduleExecutions.mockReturnValue({
      data: { data: mockExecutions, total: 4 } as ScheduleExecutionsResponse,
      isLoading: false,
      error: null,
      refetch: jest.fn(),
    });

    mockUseScheduleStats.mockReturnValue({
      data: mockStats,
      isLoading: false,
      error: null,
    });

    mockUseUpdateSchedule.mockReturnValue({
      mutate: jest.fn(),
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });

    mockUsePauseSchedule.mockReturnValue({
      mutate: jest.fn(),
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseResumeSchedule.mockReturnValue({
      mutate: jest.fn(),
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseTriggerSchedule.mockReturnValue({
      mutate: jest.fn(),
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseDeleteSchedule.mockReturnValue({
      mutate: jest.fn(),
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockConfirm.mockReturnValue(true);
  });

  // ============================================
  // Loading State Tests
  // ============================================

  describe("Loading State", () => {
    it("shows skeleton loading state while schedule is loading", () => {
      mockUseSchedule.mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
        refetch: jest.fn(),
      });

      render(<ScheduleDetailPage />, { wrapper });

      const skeletons = document.querySelectorAll('[class*="animate-pulse"]');
      expect(skeletons.length).toBeGreaterThan(0);
    });
  });

  // ============================================
  // Error State Tests
  // ============================================

  describe("Error State", () => {
    it("shows the not-found message, a back link, and a Retry button when the schedule fails to load", () => {
      mockUseSchedule.mockReturnValue({
        data: undefined,
        isLoading: false,
        error: new Error("Schedule not found"),
        refetch: jest.fn(),
      });

      render(<ScheduleDetailPage />, { wrapper });

      expect(screen.getByText(/Schedule Not Found/i)).toBeInTheDocument();
      expect(screen.getByText(/Back to Schedules/i)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /Retry/i })).toBeInTheDocument();
    });
  });

  // ============================================
  // Schedule Details Rendering Tests
  // ============================================

  describe("Schedule Details", () => {
    it("renders the schedule's name, description, cron, timezone, target, and input", () => {
      render(<ScheduleDetailPage />, { wrapper });

      // "Daily Backup" may appear multiple times (header and card title).
      expect(screen.getAllByText("Daily Backup").length).toBeGreaterThan(0);
      expect(screen.getByText("Runs daily backup workflow for all services")).toBeInTheDocument();
      expect(screen.getByText("Active")).toBeInTheDocument();
      expect(screen.getByText("0 0 0 * * * *")).toBeInTheDocument();
      expect(screen.getByText("UTC")).toBeInTheDocument();
      expect(screen.getByText("workflow")).toBeInTheDocument();
      expect(screen.getByText("backup-workflow")).toBeInTheDocument();
      // "2" appears multiple times (max_concurrent and skipped_executions).
      expect(screen.getAllByText(/2/).length).toBeGreaterThan(0);
      expect(screen.getByText(/"bucket"/)).toBeInTheDocument();
    });

    it("renders Paused badge for disabled schedule", () => {
      mockUseSchedule.mockReturnValue({
        data: { ...mockSchedule, enabled: false },
        isLoading: false,
        error: null,
        refetch: jest.fn(),
      });

      render(<ScheduleDetailPage />, { wrapper });

      expect(screen.getByText("Paused")).toBeInTheDocument();
    });
  });

  // ============================================
  // Statistics Card Tests
  // ============================================

  describe("Statistics", () => {
    it("renders total, successful, failed, and skipped execution counts", () => {
      render(<ScheduleDetailPage />, { wrapper });

      expect(screen.getByText("100")).toBeInTheDocument();
      expect(screen.getByText("95")).toBeInTheDocument();
      // "3" and "2" may also appear elsewhere (e.g. max_concurrent), so just
      // check they exist.
      expect(screen.getAllByText("3").length).toBeGreaterThan(0);
      expect(screen.getAllByText("2").length).toBeGreaterThan(0);
    });
  });

  // ============================================
  // Execution History Tests
  // ============================================

  describe("Execution History", () => {
    it("renders execution history section", () => {
      render(<ScheduleDetailPage />, { wrapper });

      expect(screen.getByText(/Execution History/i)).toBeInTheDocument();
    });

    it.each(["completed", "failed", "running", "skipped"])(
      "renders a %s execution row",
      (status) => {
        render(<ScheduleDetailPage />, { wrapper });

        expect(screen.getAllByText(new RegExp(status, "i")).length).toBeGreaterThan(0);
      },
    );

    it("displays error message for failed executions", () => {
      render(<ScheduleDetailPage />, { wrapper });

      expect(screen.getByText(/Connection timeout/i)).toBeInTheDocument();
    });

    it("renders workflow link when workflow_id exists", () => {
      render(<ScheduleDetailPage />, { wrapper });

      // Check for workflow link
      const workflowLinks = screen.getAllByRole("link");
      const workflowLink = workflowLinks.find((link) =>
        link.getAttribute("href")?.includes("workflows"),
      );
      expect(workflowLink).toBeTruthy();
    });
  });

  // ============================================
  // Execution History Filtering Tests
  // ============================================

  describe("Execution History Filtering", () => {
    it("renders status filter dropdown", () => {
      render(<ScheduleDetailPage />, { wrapper });

      // Find filter dropdown in execution history section
      const comboboxes = screen.getAllByRole("combobox");
      expect(comboboxes.length).toBeGreaterThan(0);
    });
  });

  // ============================================
  // Action Button Tests
  // ============================================

  describe("Action Buttons", () => {
    it("renders the Trigger, Pause, Edit, and Delete buttons for an enabled schedule", () => {
      render(<ScheduleDetailPage />, { wrapper });

      expect(screen.getByRole("button", { name: /Trigger/i })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /Pause/i })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /Edit/i })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /Delete/i })).toBeInTheDocument();
    });

    it("renders Resume button for disabled schedule", () => {
      mockUseSchedule.mockReturnValue({
        data: { ...mockSchedule, enabled: false },
        isLoading: false,
        error: null,
        refetch: jest.fn(),
      });

      render(<ScheduleDetailPage />, { wrapper });

      expect(screen.getByRole("button", { name: /Resume/i })).toBeInTheDocument();
    });

    it("calls pauseMutation when Pause is clicked", async () => {
      const mockPauseMutate = jest.fn();
      mockUsePauseSchedule.mockReturnValue({
        mutate: mockPauseMutate,
        mutateAsync: jest.fn(),
        isPending: false,
      });

      render(<ScheduleDetailPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Pause/i }));

      await waitFor(() => {
        expect(mockPauseMutate).toHaveBeenCalledWith("sched_123");
      });
    });

    it("calls resumeMutation when Resume is clicked", async () => {
      mockUseSchedule.mockReturnValue({
        data: { ...mockSchedule, enabled: false },
        isLoading: false,
        error: null,
        refetch: jest.fn(),
      });

      const mockResumeMutate = jest.fn();
      mockUseResumeSchedule.mockReturnValue({
        mutate: mockResumeMutate,
        mutateAsync: jest.fn(),
        isPending: false,
      });

      render(<ScheduleDetailPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Resume/i }));

      await waitFor(() => {
        expect(mockResumeMutate).toHaveBeenCalledWith("sched_123");
      });
    });

    it("calls triggerMutation when Trigger is clicked and confirmed", async () => {
      const mockTriggerMutate = jest.fn();
      mockUseTriggerSchedule.mockReturnValue({
        mutate: mockTriggerMutate,
        mutateAsync: jest.fn(),
        isPending: false,
      });
      mockConfirm.mockReturnValue(true);

      render(<ScheduleDetailPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Trigger/i }));

      await waitFor(() => {
        expect(mockTriggerMutate).toHaveBeenCalledWith("sched_123");
      });
    });

    it("navigates to schedules list after successful delete", async () => {
      // deleteMutation.mutate is called with scheduleId and options including onSuccess
      const mockDeleteMutate = jest.fn().mockImplementation((_id, options) => {
        // Simulate onSuccess callback
        if (options?.onSuccess) {
          options.onSuccess();
        }
      });
      mockUseDeleteSchedule.mockReturnValue({
        mutate: mockDeleteMutate,
        mutateAsync: jest.fn(),
        isPending: false,
      });
      mockConfirm.mockReturnValue(true);

      render(<ScheduleDetailPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Delete/i }));

      await waitFor(() => {
        expect(mockDeleteMutate).toHaveBeenCalled();
      });

      await waitFor(() => {
        expect(mockPush).toHaveBeenCalledWith("/durable/schedules");
      });
    });
  });

  // ============================================
  // Edit Schedule Dialog Tests
  // ============================================

  describe("Edit Schedule Dialog", () => {
    it("opens dialog when Edit button is clicked", async () => {
      render(<ScheduleDetailPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Edit/i }));

      await waitFor(() => {
        expect(screen.getByRole("dialog")).toBeInTheDocument();
      });
    });

    it("pre-populates form with current schedule values", async () => {
      render(<ScheduleDetailPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Edit/i }));

      await waitFor(() => {
        // Check that description is pre-filled
        const descriptionInput = screen.getByDisplayValue(
          "Runs daily backup workflow for all services",
        );
        expect(descriptionInput).toBeInTheDocument();
      });
    });

    it("calls updateMutation on submit", async () => {
      const mockUpdateMutate = jest.fn().mockResolvedValue({});
      mockUseUpdateSchedule.mockReturnValue({
        mutateAsync: mockUpdateMutate,
        isPending: false,
      });

      render(<ScheduleDetailPage />, { wrapper });

      // Open dialog
      fireEvent.click(screen.getByRole("button", { name: /Edit/i }));

      await waitFor(() => {
        expect(screen.getByRole("dialog")).toBeInTheDocument();
      });

      // Submit form
      const saveButton = screen.getByRole("button", { name: /Save/i });
      fireEvent.click(saveButton);

      await waitFor(() => {
        expect(mockUpdateMutate).toHaveBeenCalled();
      });
    });
  });

  // ============================================
  // Navigation Tests
  // ============================================

  describe("Navigation", () => {
    it("Back link has correct href", () => {
      render(<ScheduleDetailPage />, { wrapper });

      const backLink = screen.getByRole("link", { name: /Back to Schedules/i });
      expect(backLink).toHaveAttribute("href", "/durable/schedules");
    });
  });
});
