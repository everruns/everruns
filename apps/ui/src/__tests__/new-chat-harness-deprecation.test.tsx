import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { NewPlaygroundChatForm } from "@/components/chat/new-chat-form";

const mockCreate = jest.fn().mockResolvedValue({ id: "session_legacy" });
jest.mock("next/navigation", () => ({ useRouter: () => ({ push: jest.fn() }) }));
jest.mock("@/hooks/use-sessions", () => ({
  useCreateSession: () => ({ mutateAsync: mockCreate, isPending: false }),
}));
jest.mock("@/hooks", () => ({
  useAgents: () => ({ data: [], isLoading: false }),
  useHarnesses: () => ({
    data: [
      {
        id: "legacy",
        name: "generic",
        display_name: "Generic — deprecated",
        is_built_in: true,
        tags: ["deprecated"],
      },
    ],
    isLoading: false,
  }),
}));
jest.mock("@/hooks/use-providers", () => ({
  useModels: () => ({
    data: [{ enabled: true, healthy: true, capabilities: ["chat"] }],
    isLoading: false,
  }),
  useProvidersConfig: () => ({ data: { policies: {} }, isLoading: false }),
}));

describe("Playground harness deprecation", () => {
  beforeEach(() => mockCreate.mockClear());

  it("offers legacy harnesses explicitly", async () => {
    render(<NewPlaygroundChatForm endUserId="virtual-user" />);
    fireEvent.click(screen.getByRole("combobox", { name: "Chat counterpart" }));
    expect(screen.queryByRole("option", { name: "Generic — deprecated" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show deprecated" }));
    const legacyOption = screen.getByRole("option", { name: "Generic — deprecated" });
    fireEvent.pointerDown(legacyOption);
    fireEvent.click(legacyOption);
    expect(screen.getByRole("combobox")).toHaveTextContent("Generic — deprecated");
    fireEvent.click(screen.getByRole("combobox"));
    fireEvent.click(screen.getByRole("button", { name: "Hide deprecated" }));
    expect(screen.getByRole("option", { name: "Generic — deprecated" })).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("combobox"), { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: /Start .*chat/ }));
    await waitFor(() =>
      expect(mockCreate).toHaveBeenCalledWith({
        request: expect.objectContaining({ harness_name: "generic", source: "playground" }),
      }),
    );
  });
});
