import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import SchedulesPage from "@/app/(main)/durable/schedules/page";
import type { DurableSchedule, SchedulesResponse } from "@/lib/api/types";

// Mock schedule data
const mockSchedules: DurableSchedule[] = [
  {
    id: "sched_123",
    name: "Daily Backup",
    description: "Runs daily backup workflow",
    cron_expression: "0 0 0 * * * *",
    timezone: "UTC",
    target: {
      type: "workflow",
      name: "backup-workflow",
      input: { bucket: "backups" },
    },
    enabled: true,
    max_concurrent: 1,
    catch_up_missed: false,
    max_catch_up: 10,
    last_triggered_at: "2024-01-15T00:00:00Z",
    next_trigger_at: "2024-01-16T00:00:00Z",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
  {
    id: "sched_456",
    name: "Hourly Cleanup",
    description: "Cleans up temporary files",
    cron_expression: "0 0 * * * * *",
    timezone: "America/New_York",
    target: {
      type: "activity",
      name: "cleanup-activity",
      input: {},
    },
    enabled: false,
    catch_up_missed: true,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
];

// Mock hooks
const mockUseSchedules = jest.fn();
const mockUseCreateSchedule = jest.fn();
const mockUsePauseSchedule = jest.fn();
const mockUseResumeSchedule = jest.fn();
const mockUseTriggerSchedule = jest.fn();
const mockUseDeleteSchedule = jest.fn();

jest.mock("@/hooks/use-durable", () => ({
  useSchedules: () => mockUseSchedules(),
  useCreateSchedule: () => mockUseCreateSchedule(),
  usePauseSchedule: () => mockUsePauseSchedule(),
  useResumeSchedule: () => mockUseResumeSchedule(),
  useTriggerSchedule: () => mockUseTriggerSchedule(),
  useDeleteSchedule: () => mockUseDeleteSchedule(),
}));

// Mock Next.js navigation
const mockPush = jest.fn();
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush }),
  usePathname: () => "/durable/schedules",
}));

// Mock window.confirm for delete/trigger actions
const mockConfirm = jest.fn();
window.confirm = mockConfirm;

describe("SchedulesPage", () => {
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
    mockUseSchedules.mockReturnValue({
      data: { data: mockSchedules, total: 2 } as SchedulesResponse,
      isLoading: false,
      error: null,
      refetch: jest.fn(),
    });

    mockUseCreateSchedule.mockReturnValue({
      mutate: jest.fn(),
      mutateAsync: jest.fn(),
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
    it("shows skeleton loading state while schedules are loading", () => {
      mockUseSchedules.mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
        refetch: jest.fn(),
      });

      render(<SchedulesPage />, { wrapper });

      const skeletons = document.querySelectorAll('[class*="animate-pulse"]');
      expect(skeletons.length).toBeGreaterThan(0);
    });
  });

  // ============================================
  // Error State Tests
  // ============================================

  describe("Error State", () => {
    it("shows error message when schedules fail to load", () => {
      mockUseSchedules.mockReturnValue({
        data: undefined,
        isLoading: false,
        error: new Error("Network error"),
        refetch: jest.fn(),
      });

      render(<SchedulesPage />, { wrapper });

      // Component shows "Unable to Load Schedules" for error state
      expect(screen.getByText(/Unable to Load Schedules/i)).toBeInTheDocument();
    });

    it("shows retry button on error", () => {
      mockUseSchedules.mockReturnValue({
        data: undefined,
        isLoading: false,
        error: new Error("Network error"),
        refetch: jest.fn(),
      });

      render(<SchedulesPage />, { wrapper });

      expect(screen.getByRole("button", { name: /Retry/i })).toBeInTheDocument();
    });

    it("calls refetch when retry button is clicked", () => {
      const mockRefetch = jest.fn();
      mockUseSchedules.mockReturnValue({
        data: undefined,
        isLoading: false,
        error: new Error("Network error"),
        refetch: mockRefetch,
      });

      render(<SchedulesPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /Retry/i }));
      expect(mockRefetch).toHaveBeenCalled();
    });
  });

  // ============================================
  // Empty State Tests
  // ============================================

  describe("Empty State", () => {
    it("shows empty state when no schedules exist", () => {
      mockUseSchedules.mockReturnValue({
        data: { data: [], total: 0 },
        isLoading: false,
        error: null,
        refetch: jest.fn(),
      });

      render(<SchedulesPage />, { wrapper });

      expect(screen.getByText(/No scheduled tasks/i)).toBeInTheDocument();
    });
  });

  // ============================================
  // Schedule List Rendering Tests
  // ============================================

  describe("Schedule List Rendering", () => {
    it("renders page header with title", () => {
      render(<SchedulesPage />, { wrapper });

      expect(screen.getByText("Scheduled Tasks")).toBeInTheDocument();
    });

    it("renders schedule table with correct headers", () => {
      render(<SchedulesPage />, { wrapper });

      expect(screen.getByText("Schedule")).toBeInTheDocument();
      expect(screen.getByText("Status")).toBeInTheDocument();
      expect(screen.getByText("Cron")).toBeInTheDocument();
      expect(screen.getByText("Target")).toBeInTheDocument();
    });

    it("renders schedule rows with correct names", () => {
      render(<SchedulesPage />, { wrapper });

      expect(screen.getByText("Daily Backup")).toBeInTheDocument();
      expect(screen.getByText("Hourly Cleanup")).toBeInTheDocument();
    });

    it("shows Active badge for enabled schedules", () => {
      render(<SchedulesPage />, { wrapper });

      const activeBadge = screen.getByText("Active");
      expect(activeBadge).toBeInTheDocument();
    });

    it("shows Paused badge for disabled schedules", () => {
      render(<SchedulesPage />, { wrapper });

      const pausedBadge = screen.getByText("Paused");
      expect(pausedBadge).toBeInTheDocument();
    });

    it("displays cron expression", () => {
      render(<SchedulesPage />, { wrapper });

      expect(screen.getByText("0 0 0 * * * *")).toBeInTheDocument();
      expect(screen.getByText("0 0 * * * * *")).toBeInTheDocument();
    });

    it("displays target type and name", () => {
      render(<SchedulesPage />, { wrapper });

      expect(screen.getByText("workflow")).toBeInTheDocument();
      expect(screen.getByText("backup-workflow")).toBeInTheDocument();
      expect(screen.getByText("activity")).toBeInTheDocument();
      expect(screen.getByText("cleanup-activity")).toBeInTheDocument();
    });
  });

  // ============================================
  // Search and Filter Tests
  // ============================================

  describe("Search and Filter", () => {
    it("renders search input", () => {
      render(<SchedulesPage />, { wrapper });

      expect(screen.getByPlaceholderText(/Search/i)).toBeInTheDocument();
    });

    it("renders status filter dropdown", () => {
      render(<SchedulesPage />, { wrapper });

      // Look for the select trigger
      const statusFilter = screen.getByRole("combobox");
      expect(statusFilter).toBeInTheDocument();
    });
  });

  // ============================================
  // Action Button Tests
  // ============================================

  describe("Action Buttons", () => {
    it("renders action buttons for each schedule", () => {
      render(<SchedulesPage />, { wrapper });

      // Should have multiple action buttons (trigger, pause/resume, settings, delete)
      const buttons = screen.getAllByRole("button");
      expect(buttons.length).toBeGreaterThan(2);
    });

    it("calls pauseMutation when Pause button is clicked", async () => {
      const mockPauseMutate = jest.fn();
      mockUsePauseSchedule.mockReturnValue({
        mutate: mockPauseMutate,
        mutateAsync: jest.fn(),
        isPending: false,
      });

      render(<SchedulesPage />, { wrapper });

      // Find pause icon button by SVG class
      const buttons = screen.getAllByRole("button");
      const pauseButton = buttons.find((btn) => btn.querySelector(".lucide-pause"));
      expect(pauseButton).toBeTruthy();
      if (pauseButton) {
        fireEvent.click(pauseButton);
        await waitFor(() => {
          expect(mockPauseMutate).toHaveBeenCalled();
        });
      }
    });

    it("calls resumeMutation when Resume button is clicked", async () => {
      const mockResumeMutate = jest.fn();
      mockUseResumeSchedule.mockReturnValue({
        mutate: mockResumeMutate,
        mutateAsync: jest.fn(),
        isPending: false,
      });

      render(<SchedulesPage />, { wrapper });

      // Find resume icon button by SVG class
      const buttons = screen.getAllByRole("button");
      const resumeButton = buttons.find((btn) => btn.querySelector(".lucide-play"));
      expect(resumeButton).toBeTruthy();
      if (resumeButton) {
        fireEvent.click(resumeButton);
        await waitFor(() => {
          expect(mockResumeMutate).toHaveBeenCalled();
        });
      }
    });

    it("calls triggerMutation when Trigger button is clicked and confirmed", async () => {
      const mockTriggerMutate = jest.fn();
      mockUseTriggerSchedule.mockReturnValue({
        mutate: mockTriggerMutate,
        mutateAsync: jest.fn(),
        isPending: false,
      });
      mockConfirm.mockReturnValue(true);

      render(<SchedulesPage />, { wrapper });

      // Find trigger icon button by SVG class
      const buttons = screen.getAllByRole("button");
      const triggerButton = buttons.find((btn) => btn.querySelector(".lucide-zap"));
      expect(triggerButton).toBeTruthy();
      if (triggerButton) {
        fireEvent.click(triggerButton);
        await waitFor(() => {
          expect(mockTriggerMutate).toHaveBeenCalled();
        });
      }
    });

    it("does not call triggerMutation when confirmation is cancelled", async () => {
      const mockTriggerMutate = jest.fn();
      mockUseTriggerSchedule.mockReturnValue({
        mutate: mockTriggerMutate,
        mutateAsync: jest.fn(),
        isPending: false,
      });
      mockConfirm.mockReturnValue(false);

      render(<SchedulesPage />, { wrapper });

      // Find trigger icon button
      const buttons = screen.getAllByRole("button");
      const triggerButton = buttons.find((btn) => btn.querySelector(".lucide-zap"));
      if (triggerButton) {
        fireEvent.click(triggerButton);
        expect(mockTriggerMutate).not.toHaveBeenCalled();
      }
    });

    it("calls deleteMutation when Delete button is clicked and confirmed", async () => {
      const mockDeleteMutate = jest.fn();
      mockUseDeleteSchedule.mockReturnValue({
        mutate: mockDeleteMutate,
        mutateAsync: jest.fn(),
        isPending: false,
      });
      mockConfirm.mockReturnValue(true);

      render(<SchedulesPage />, { wrapper });

      // Find delete button by destructive class
      const buttons = screen.getAllByRole("button");
      const deleteButton = buttons.find((btn) => btn.className.includes("text-destructive"));
      expect(deleteButton).toBeTruthy();
      if (deleteButton) {
        fireEvent.click(deleteButton);
        await waitFor(() => {
          expect(mockDeleteMutate).toHaveBeenCalled();
        });
      }
    });
  });

  // ============================================
  // Create Schedule Dialog Tests
  // ============================================

  describe("Create Schedule Dialog", () => {
    it("renders New Schedule button", () => {
      render(<SchedulesPage />, { wrapper });

      expect(screen.getByRole("button", { name: /New Schedule/i })).toBeInTheDocument();
    });

    it("opens dialog when New Schedule button is clicked", async () => {
      render(<SchedulesPage />, { wrapper });

      const createButton = screen.getByRole("button", { name: /New Schedule/i });
      fireEvent.click(createButton);

      await waitFor(() => {
        expect(screen.getByRole("dialog")).toBeInTheDocument();
      });
    });

    it("renders form fields in create dialog", async () => {
      render(<SchedulesPage />, { wrapper });

      const createButton = screen.getByRole("button", { name: /New Schedule/i });
      fireEvent.click(createButton);

      await waitFor(() => {
        // Use id-based selector to avoid matching table header
        const nameInput = document.getElementById("name");
        expect(nameInput).toBeInTheDocument();
        const cronInput = document.getElementById("cron");
        expect(cronInput).toBeInTheDocument();
      });
    });

    it("calls createMutation with correct data on submit", async () => {
      const mockCreateMutate = jest.fn().mockResolvedValue({
        id: "sched_new",
        name: "Test Schedule",
      });
      mockUseCreateSchedule.mockReturnValue({
        mutateAsync: mockCreateMutate,
        isPending: false,
      });

      render(<SchedulesPage />, { wrapper });

      // Open dialog
      fireEvent.click(screen.getByRole("button", { name: /New Schedule/i }));

      await waitFor(() => {
        expect(screen.getByRole("dialog")).toBeInTheDocument();
      });

      // Fill form using id-based selectors
      const nameInput = document.getElementById("name") as HTMLInputElement;
      fireEvent.change(nameInput, { target: { value: "Test Schedule" } });

      const targetNameInput = document.getElementById("targetName") as HTMLInputElement;
      fireEvent.change(targetNameInput, { target: { value: "test-workflow" } });

      // Submit form - button text is "Create Schedule"
      const submitButton = screen.getByRole("button", { name: /Create Schedule/i });
      fireEvent.click(submitButton);

      await waitFor(() => {
        expect(mockCreateMutate).toHaveBeenCalled();
      });
    });

    it("sends correct request shape to API on submit", async () => {
      const mockCreateMutate = jest.fn().mockResolvedValue({
        id: "sched_new",
        name: "My Schedule",
      });
      mockUseCreateSchedule.mockReturnValue({
        mutateAsync: mockCreateMutate,
        isPending: false,
      });

      render(<SchedulesPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /New Schedule/i }));

      await waitFor(() => {
        expect(screen.getByRole("dialog")).toBeInTheDocument();
      });

      const nameInput = document.getElementById("name") as HTMLInputElement;
      fireEvent.change(nameInput, { target: { value: "My Schedule" } });

      const targetNameInput = document.getElementById("targetName") as HTMLInputElement;
      fireEvent.change(targetNameInput, { target: { value: "my-workflow" } });

      const submitButton = screen.getByRole("button", { name: /Create Schedule/i });
      fireEvent.click(submitButton);

      await waitFor(() => {
        expect(mockCreateMutate).toHaveBeenCalledWith({
          name: "My Schedule",
          description: undefined,
          cron_expression: "0 * * * * * *",
          target: {
            type: "workflow",
            name: "my-workflow",
            input: {},
          },
          enabled: true,
        });
      });
    });

    it("displays error message when schedule creation fails", async () => {
      const mockCreateMutate = jest.fn().mockRejectedValue(
        new Error("API Error: 400 Bad Request"),
      );
      mockUseCreateSchedule.mockReturnValue({
        mutateAsync: mockCreateMutate,
        isPending: false,
      });

      render(<SchedulesPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /New Schedule/i }));

      await waitFor(() => {
        expect(screen.getByRole("dialog")).toBeInTheDocument();
      });

      const nameInput = document.getElementById("name") as HTMLInputElement;
      fireEvent.change(nameInput, { target: { value: "Test Schedule" } });

      const targetNameInput = document.getElementById("targetName") as HTMLInputElement;
      fireEvent.change(targetNameInput, { target: { value: "test-workflow" } });

      const submitButton = screen.getByRole("button", { name: /Create Schedule/i });
      fireEvent.click(submitButton);

      await waitFor(() => {
        expect(screen.getByText("API Error: 400 Bad Request")).toBeInTheDocument();
      });
    });

    it("displays error for invalid JSON in target input", async () => {
      mockUseCreateSchedule.mockReturnValue({
        mutateAsync: jest.fn(),
        isPending: false,
      });

      render(<SchedulesPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /New Schedule/i }));

      await waitFor(() => {
        expect(screen.getByRole("dialog")).toBeInTheDocument();
      });

      const nameInput = document.getElementById("name") as HTMLInputElement;
      fireEvent.change(nameInput, { target: { value: "Test Schedule" } });

      const targetNameInput = document.getElementById("targetName") as HTMLInputElement;
      fireEvent.change(targetNameInput, { target: { value: "test-workflow" } });

      const targetInputField = document.getElementById("targetInput") as HTMLTextAreaElement;
      fireEvent.change(targetInputField, { target: { value: "not valid json" } });

      const submitButton = screen.getByRole("button", { name: /Create Schedule/i });
      fireEvent.click(submitButton);

      await waitFor(() => {
        expect(screen.getByText("Invalid JSON in target input")).toBeInTheDocument();
      });
    });

    it("does not close dialog when creation fails", async () => {
      const mockCreateMutate = jest.fn().mockRejectedValue(
        new Error("Server error"),
      );
      mockUseCreateSchedule.mockReturnValue({
        mutateAsync: mockCreateMutate,
        isPending: false,
      });

      render(<SchedulesPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /New Schedule/i }));

      await waitFor(() => {
        expect(screen.getByRole("dialog")).toBeInTheDocument();
      });

      const nameInput = document.getElementById("name") as HTMLInputElement;
      fireEvent.change(nameInput, { target: { value: "Test Schedule" } });

      const targetNameInput = document.getElementById("targetName") as HTMLInputElement;
      fireEvent.change(targetNameInput, { target: { value: "test-workflow" } });

      const submitButton = screen.getByRole("button", { name: /Create Schedule/i });
      fireEvent.click(submitButton);

      await waitFor(() => {
        expect(screen.getByText("Server error")).toBeInTheDocument();
      });

      // Dialog should still be open
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    it("disables Cancel button while creation is pending", async () => {
      mockUseCreateSchedule.mockReturnValue({
        mutateAsync: jest.fn().mockImplementation(() => new Promise(() => {})),
        isPending: true,
      });

      render(<SchedulesPage />, { wrapper });

      fireEvent.click(screen.getByRole("button", { name: /New Schedule/i }));

      await waitFor(() => {
        expect(screen.getByRole("dialog")).toBeInTheDocument();
      });

      const cancelButton = screen.getByRole("button", { name: /Cancel/i });
      expect(cancelButton).toBeDisabled();
    });
  });

  // ============================================
  // Refresh Tests
  // ============================================

  describe("Refresh", () => {
    it("renders refresh button", () => {
      render(<SchedulesPage />, { wrapper });

      // Look for refresh icon button by SVG class
      const buttons = screen.getAllByRole("button");
      const refreshButton = buttons.find((btn) => btn.querySelector(".lucide-refresh-cw"));
      expect(refreshButton).toBeTruthy();
    });

    it("calls refetch when refresh button is clicked", () => {
      const mockRefetch = jest.fn();
      mockUseSchedules.mockReturnValue({
        data: { data: mockSchedules, total: 2 },
        isLoading: false,
        error: null,
        refetch: mockRefetch,
      });

      render(<SchedulesPage />, { wrapper });

      // Find refresh button by SVG class
      const buttons = screen.getAllByRole("button");
      const refreshButton = buttons.find((btn) => btn.querySelector(".lucide-refresh-cw"));
      expect(refreshButton).toBeTruthy();
      if (refreshButton) {
        fireEvent.click(refreshButton);
        expect(mockRefetch).toHaveBeenCalled();
      }
    });
  });
});
