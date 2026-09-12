import { render, screen } from "@testing-library/react";
import { NoIntelligenceNotice } from "@/components/chat/no-intelligence-notice";

const modelsState = {
  data: [] as { enabled: boolean; healthy: boolean; capabilities: string[] }[] | undefined,
  isLoading: false,
  isError: false,
};
const configState = {
  data: { policies: {} as Record<string, boolean> } as
    | { policies: Record<string, boolean> }
    | undefined,
  isLoading: false,
};

jest.mock("@/hooks/use-providers", () => ({
  useModels: () => modelsState,
  useProvidersConfig: () => configState,
}));

function chatModel(overrides: Partial<{ enabled: boolean; healthy: boolean }> = {}) {
  return { enabled: true, healthy: true, capabilities: ["chat"], ...overrides };
}

describe("NoIntelligenceNotice", () => {
  beforeEach(() => {
    modelsState.data = [];
    modelsState.isLoading = false;
    modelsState.isError = false;
    configState.data = { policies: {} };
    configState.isLoading = false;
  });

  it("tells the user chat has no model and offers the fix when they may manage providers", () => {
    configState.data = { policies: { "provider.manage": true } };

    render(<NoIntelligenceNotice />);

    expect(screen.getByText("No intelligence available")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /manage providers & models/i })).toHaveAttribute(
      "href",
      "/settings/providers",
    );
  });

  it("points a member at an admin instead of a link they cannot act on", () => {
    configState.data = { policies: { "provider.manage": false } };

    render(<NoIntelligenceNotice />);

    expect(screen.getByText("No intelligence available")).toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(screen.getByText(/ask an organisation owner or admin/i)).toBeInTheDocument();
  });

  it("stays silent once an enabled, healthy chat model exists", () => {
    modelsState.data = [chatModel()];

    render(<NoIntelligenceNotice />);

    expect(screen.queryByText("No intelligence available")).not.toBeInTheDocument();
  });

  it("still warns when the only models are unhealthy, disabled, or embeddings", () => {
    modelsState.data = [
      chatModel({ healthy: false }),
      chatModel({ enabled: false }),
      { enabled: true, healthy: true, capabilities: ["embeddings"] },
    ];

    render(<NoIntelligenceNotice />);

    expect(screen.getByText("No intelligence available")).toBeInTheDocument();
  });

  it("renders nothing while loading or when the model list could not be read", () => {
    modelsState.isLoading = true;
    const { rerender } = render(<NoIntelligenceNotice />);
    expect(screen.queryByText("No intelligence available")).not.toBeInTheDocument();

    modelsState.isLoading = false;
    modelsState.isError = true;
    modelsState.data = undefined;
    rerender(<NoIntelligenceNotice />);
    expect(screen.queryByText("No intelligence available")).not.toBeInTheDocument();
  });
});
