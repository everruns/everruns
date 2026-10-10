import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ChatGptConnectionCard } from "@/components/providers/chatgpt-connection";
const mockDisconnect = jest.fn();
const mockRefetch = jest.fn();
const mockInvalidate = jest.fn();
let mockConnection = {
  status: "connected",
  email: "me@example.com",
  host_id: "urn:uuid:test",
  error: null,
};
jest.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ invalidateQueries: mockInvalidate }),
}));
jest.mock("@/hooks/use-chatgpt-connection", () => ({
  useChatGptConnection: () => ({
    data: mockConnection,
    refetch: mockRefetch,
    isLoading: false,
    isError: false,
  }),
}));
jest.mock("@/lib/api/chatgpt", () => ({
  CHATGPT_USAGE_URL: "https://chatgpt.com/settings/usage",
  disconnectChatGpt: (...args: unknown[]) => mockDisconnect(...args),
  startChatGptLogin: jest.fn(),
  importChatGptConnection: jest.fn(),
}));
describe("ChatGPT account settings", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    localStorage.clear();
    mockDisconnect.mockResolvedValue(undefined);
    mockConnection = {
      status: "connected",
      email: "me@example.com",
      host_id: "urn:uuid:test",
      error: null,
    };
  });
  it("shows personal ownership, usage and the first-sign-in notice once", async () => {
    const view = render(<ChatGptConnectionCard providerId="provider-personal" />);
    expect(screen.getByText("Only you")).toBeInTheDocument();
    expect(screen.getByText("me@example.com")).toBeInTheDocument();

    fireEvent.click(await screen.findByRole("button", { name: "Got it" }));
    expect(screen.getByRole("link", { name: /Manage usage/ })).toHaveAttribute(
      "href",
      "https://chatgpt.com/settings/usage",
    );
    expect(localStorage.getItem("chatgpt-plan-notice:me@example.com")).toBe("seen");
    view.unmount();
    render(<ChatGptConnectionCard providerId="provider-new" />);
    expect(screen.queryByText("You’re using your ChatGPT plan")).not.toBeInTheDocument();
  });
  it("retains a useful error when revocation cannot be confirmed", async () => {
    localStorage.setItem("chatgpt-plan-notice:me@example.com", "seen");
    mockDisconnect.mockRejectedValue(new Error("revocation not confirmed"));
    render(<ChatGptConnectionCard providerId="provider-personal" />);
    fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("credentials were retained"),
    );
    expect(mockDisconnect).toHaveBeenCalledWith("provider-personal");
  });
});
