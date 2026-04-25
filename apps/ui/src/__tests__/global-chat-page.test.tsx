import { render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import GlobalChatPage from "@/app/(main)/chat/page";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { useGlobalChat } from "@/hooks/use-global-chat";

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: jest.fn(),
}));

jest.mock("@/hooks/use-global-chat", () => ({
  useGlobalChat: jest.fn(),
}));

jest.mock("@/components/chat/chat-panel", () => ({
  ChatPanel: () => <div>chat-panel</div>,
}));

jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  SessionProvider: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

jest.mock("@/components/ui/experimental-badge", () => ({
  ExperimentalPageBadge: () => <div>badge</div>,
}));

const mockUseFeatureFlag = jest.mocked(useFeatureFlag);
const mockUseGlobalChat = jest.mocked(useGlobalChat);

describe("GlobalChatPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("does not initialize global chat when feature flag is disabled", () => {
    mockUseFeatureFlag.mockReturnValue(false);

    render(<GlobalChatPage />);

    expect(screen.getByText("Global Chat is not enabled")).toBeInTheDocument();
    expect(mockUseGlobalChat).not.toHaveBeenCalled();
  });

  it("initializes global chat when feature flag is enabled", () => {
    mockUseFeatureFlag.mockReturnValue(true);
    mockUseGlobalChat.mockReturnValue({
      sessionId: "session_123",
      isLoading: false,
      error: null,
    });

    render(<GlobalChatPage />);

    expect(mockUseGlobalChat).toHaveBeenCalledTimes(1);
    expect(screen.getByText("chat-panel")).toBeInTheDocument();
  });
});
