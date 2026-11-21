import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { QualityMonitoringSettings } from "@/components/organizations/quality-monitoring-settings";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
import type { Organization } from "@/lib/api/types";

const mockSave = jest.fn();
const mockReview = jest.fn();
const mockScores = jest.fn();
jest.mock("@/hooks/use-organizations", () => ({
  useUpdateOrganization: () => ({ mutateAsync: mockSave, isPending: false }),
}));
jest.mock("@/lib/api/quality-monitoring", () => ({
  getQualityMonitoring: async () => ({ today: { reserved_usd: 0, reported_usd: 0 }, checks: [] }),
  getQualityMonitoringScores: () => mockScores(),
  reviewQualityMonitoringScore: (...args: unknown[]) => mockReview(...args),
}));
jest.mock("@/components/models/model-picker", () => ({
  ModelPicker: ({
    id,
    value,
    disabled,
    onChange,
  }: {
    id: string;
    value: string;
    disabled: boolean;
    onChange: (value: string) => void;
  }) => (
    <select
      id={id}
      value={value}
      disabled={disabled}
      onChange={(event) => onChange(event.target.value)}
    >
      <option value="">Mechanical checks only</option>
      <option value="model-1">Judge</option>
    </select>
  ),
}));
const organization = { id: "org-1" } as Organization;
let adapter: PageDraftAdapter;
const mode: ChatModeState = {
  page: { route: "/settings/organization", label: "Organization" },
  pageContainer: { current: null },
  open: true,
  setOpen: () => {},
  actions: [],
  recordAction: () => {},
  registerPage: (_, value) => {
    adapter = value!;
    return () => {};
  },
};
function setup(canManage = true, adoption = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <PageChatProvider value={adoption ? mode : null}>
        <QualityMonitoringSettings organization={organization} canManage={canManage} />
      </PageChatProvider>
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  jest.clearAllMocks();
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
  mockScores.mockResolvedValue([]);
  mockSave.mockImplementation(async (patch) => ({ ...organization, ...patch }));
  mockReview.mockResolvedValue({});
});
it("defaults off and enables only after explicit Save with a zero budget", async () => {
  setup();
  expect(screen.getByRole("switch")).not.toBeChecked();
  fireEvent.click(screen.getByRole("switch"));
  expect(mockSave).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(mockSave).toHaveBeenCalledWith({
      quality_monitoring: expect.objectContaining({
        enabled: true,
        daily_budget_usd: 0,
        model_id: null,
      }),
    }),
  );
});
it("requires an explicit model for paid checks and keeps member controls read-only", async () => {
  const view = setup();
  fireEvent.click(screen.getByRole("switch"));
  fireEvent.change(screen.getByLabelText("Daily monitoring budget (USD)"), {
    target: { value: "1" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Select a monitoring model");
  expect(mockSave).not.toHaveBeenCalled();
  view.unmount();
  setup(false);
  expect(screen.getByRole("switch")).toBeDisabled();
  expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
});
it("shows insufficient evidence separately and records a human disagreement", async () => {
  mockScores.mockResolvedValue([
    {
      id: "score-1",
      scorer_key: "compaction_continuity_v1",
      status: "completed",
      pass: null,
      reason: "No usable continuation",
      session_id: "session-1",
    },
  ]);
  setup();
  expect(await screen.findByText("Insufficient evidence")).toBeInTheDocument();
  expect(screen.queryByText("Passed")).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Open trace" })).toHaveAttribute(
    "href",
    "/sessions/session-1/trace",
  );
  const saved = await mockScores.mock.results[0].value;
  mockScores.mockResolvedValue(
    saved.map((score: object) => ({ ...score, review: { agrees: false } })),
  );
  fireEvent.click(screen.getByRole("button", { name: "Disagree with assessment" }));
  await waitFor(() => expect(mockReview).toHaveBeenCalledWith("org-1", "score-1", false));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Disagree with assessment" })).toHaveAttribute(
      "aria-pressed",
      "true",
    ),
  );
});
it("Chat stages monitoring consent without saving until the person applies it", async () => {
  setup(true, true);
  const before = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(before.binding, before.revision, { enabled: true });
  });
  expect(screen.getByRole("switch")).toBeChecked();
  expect(mockSave).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() =>
    expect(mockSave).toHaveBeenCalledWith({
      quality_monitoring: expect.objectContaining({ enabled: true }),
    }),
  );
});
