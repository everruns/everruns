import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
import ChatSettingsPage from "@/app/(main)/settings/chat/page";

let mockOrganization = {
  id: "org-1",
  chat_model_id: "model-chat",
  default_model_id: "model-agent",
};
let mockCanManage = true;
const mockMutate = jest.fn();
jest.mock("@/hooks", () => ({ usePageTitle: jest.fn() }));
jest.mock("@/components/chat-mode/use-page-read-context", () => ({
  usePageReadContext: jest.fn(),
}));
jest.mock("@/providers/org-provider", () => ({ useOrg: () => ({ hasRole: () => mockCanManage }) }));
jest.mock("@/hooks/use-organizations", () => ({
  useOrganization: () => ({ data: mockOrganization, isLoading: false, error: null }),
  useUpdateOrganization: () => ({ mutateAsync: mockMutate, isPending: false }),
}));
jest.mock("@/components/models/model-picker", () => ({
  ModelPicker: ({
    value,
    onChange,
    disabled,
    organizationDefault,
    requireTools,
  }: {
    value: string;
    onChange: (v: string) => void;
    disabled: boolean;
    organizationDefault: boolean;
    requireTools: boolean;
  }) => (
    <select
      aria-label="Chat model"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      disabled={disabled}
      data-org-default={organizationDefault}
      data-tools={requireTools}
    >
      <option value="">Use agents and sessions default</option>
      <option value="model-chat">Current Chat</option>
      <option value="model-other">Another eligible model</option>
      <option value="model-concurrent">Concurrent model</option>
    </select>
  ),
}));
beforeEach(() => {
  global.structuredClone = value => JSON.parse(JSON.stringify(value));
  mockOrganization = { id: "org-1", chat_model_id: "model-chat", default_model_id: "model-agent" };
  mockCanManage = true;
  mockMutate.mockReset().mockImplementation(async (patch) => ({ ...mockOrganization, ...patch }));
});
test("selects any eligible model and saves only the independent Chat field", async () => {
  render(<ChatSettingsPage />);
  const picker = screen.getByLabelText("Chat model");
  expect(picker).toHaveAttribute("data-org-default", "true");
  expect(picker).toHaveAttribute("data-tools", "true");
  fireEvent.change(picker, { target: { value: "model-other" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(mockMutate).toHaveBeenCalledWith({ chat_model_id: "model-other" }));
  expect(mockOrganization.default_model_id).toBe("model-agent");
});
test("clearing Chat is an explicit null, not a write to agents or sessions", async () => {
  render(<ChatSettingsPage />);
  fireEvent.change(screen.getByLabelText("Chat model"), { target: { value: "" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(mockMutate).toHaveBeenCalledWith({ chat_model_id: null }));
});
test("members can view but cannot change organization Chat settings", () => {
  mockCanManage = false;
  render(<ChatSettingsPage />);
  expect(screen.getByLabelText("Chat model")).toBeDisabled();
  expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
});
test("a tenant switch discards unsaved selections", () => {
  const { rerender } = render(<ChatSettingsPage />);
  fireEvent.change(screen.getByLabelText("Chat model"), { target: { value: "model-other" } });
  mockOrganization = { id: "org-2", chat_model_id: "model-chat", default_model_id: "model-agent" };
  rerender(<ChatSettingsPage />);
  expect(screen.getByLabelText("Chat model")).toHaveValue("model-chat");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});
test("a rejected choice remains unsaved and shows the API error", async () => {
  mockMutate.mockRejectedValue(new Error("Chat model must have an available provider"));
  render(<ChatSettingsPage />);
  fireEvent.change(screen.getByLabelText("Chat model"), { target: { value: "model-other" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("available provider"));
  expect(screen.getByRole("button", { name: "Save" })).not.toBeDisabled();
});

let adapter: PageDraftAdapter;
const mode: ChatModeState = {
  page: { route: "/settings/chat", label: "Chat settings" },
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
test("stages Chat model edits in the live form and saves only after review", async () => {
  render(
    <PageChatProvider value={mode}>
      <ChatSettingsPage />
    </PageChatProvider>,
  );
  const before = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(before.binding, before.revision, { chat_model_id: "model-other" });
  });
  expect(screen.getByLabelText("Chat model")).toHaveValue("model-other");
  expect(mockMutate).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => expect(mockMutate).toHaveBeenCalledWith({ chat_model_id: "model-other" }));
  expect(mockOrganization.default_model_id).toBe("model-agent");
});
test("retains a dirty Chat model and blocks a stale Save until Discard", () => {
  const { rerender } = render(
    <PageChatProvider value={mode}>
      <ChatSettingsPage />
    </PageChatProvider>,
  );
  fireEvent.change(screen.getByLabelText("Chat model"), { target: { value: "model-other" } });
  mockOrganization = { ...mockOrganization, chat_model_id: "model-concurrent" };
  rerender(
    <PageChatProvider value={mode}>
      <ChatSettingsPage />
    </PageChatProvider>,
  );
  expect(screen.getByLabelText("Chat model")).toHaveValue("model-other");
  expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(screen.getByLabelText("Chat model")).toHaveValue("model-concurrent");
  expect(mockMutate).not.toHaveBeenCalled();
});
