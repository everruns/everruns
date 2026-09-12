import { render, screen } from "@testing-library/react";
import NewChatPageClient from "@/app/(main)/chats/new/new-chat-page-client";

const status = { isLoading: false, available: false, canManage: true };

jest.mock("@/hooks/use-intelligence", () => ({
  useIntelligenceStatus: () => status,
}));

jest.mock("@/components/chat/new-chat-form", () => ({
  NewChatForm: () => <div data-testid="new-chat-form" />,
}));

describe("New chat page", () => {
  beforeEach(() => {
    status.isLoading = false;
    status.available = false;
    status.canManage = true;
  });

  it("replaces the whole empty state when the org has no model", () => {
    render(<NewChatPageClient />);

    expect(screen.getByText("No intelligence available")).toBeInTheDocument();
    // One centred message, not two: the "New chat / pick a counterpart" frame
    // and its picker would be advice that cannot work.
    expect(screen.queryByText("New chat")).not.toBeInTheDocument();
    expect(screen.queryByTestId("new-chat-form")).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: /manage providers & models/i })).toHaveAttribute(
      "href",
      "/settings/providers",
    );
  });

  it("points a member at an admin instead of a link", () => {
    status.canManage = false;

    render(<NewChatPageClient />);

    expect(screen.queryByRole("link", { name: /manage providers/i })).not.toBeInTheDocument();
    expect(screen.getByText(/ask an organisation owner or admin/i)).toBeInTheDocument();
  });

  it("shows the normal picker once a model is available", () => {
    status.available = true;

    render(<NewChatPageClient />);

    expect(screen.getByText("New chat")).toBeInTheDocument();
    expect(screen.getByTestId("new-chat-form")).toBeInTheDocument();
    expect(screen.queryByText("No intelligence available")).not.toBeInTheDocument();
  });
});
