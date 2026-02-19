import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import ConnectionsPage from "@/app/(main)/settings/connections/page";

// Mock streamdown-message to capture rendered markdown content
jest.mock("@/components/chat/streamdown-message", () => ({
  StreamdownMessage: ({ children, className }: { children: string; className?: string }) => (
    <div data-testid="streamdown-message" className={className}>
      {children}
    </div>
  ),
  InlineStreamdownMessage: ({ children, className }: { children: string; className?: string }) => (
    <div data-testid="inline-streamdown-message" className={className}>
      {children}
    </div>
  ),
}));

// Mock next/navigation
jest.mock("next/navigation", () => ({
  useSearchParams: () => ({
    get: jest.fn().mockReturnValue(null),
  }),
}));

const mockUseUserConnections = jest.fn();
const mockUseDeleteUserConnection = jest.fn();
const mockUseConnectionProviders = jest.fn();
const mockUseCreateApiKeyConnection = jest.fn();

jest.mock("@/hooks/use-user-connections", () => ({
  useUserConnections: () => mockUseUserConnections(),
  useDeleteUserConnection: () => mockUseDeleteUserConnection(),
  useConnectionProviders: () => mockUseConnectionProviders(),
  useCreateApiKeyConnection: () => mockUseCreateApiKeyConnection(),
}));

jest.mock("@/lib/api/client", () => ({
  getBackendUrl: () => "http://localhost:8080",
}));

const mockDaytonaProvider = {
  provider_id: "daytona",
  display_name: "Daytona",
  description: "Cloud development environment",
  icon: "cloud",
  connection_type: "api_key" as const,
  form_schema: {
    fields: [
      {
        name: "api_key",
        label: "API Key",
        field_type: "password",
        required: true,
        placeholder: "Enter your Daytona API key",
        help_text: null,
      },
    ],
    instructions_markdown:
      "1. Go to [Daytona Dashboard](https://app.daytona.io)\n2. Navigate to **API Keys** in your account settings\n3. Click **Create New API Key**\n4. Copy the key and paste it below",
  },
};

describe("ConnectionsPage - Markdown Instructions", () => {
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

    mockUseUserConnections.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    mockUseConnectionProviders.mockReturnValue({
      data: [mockDaytonaProvider],
    });

    mockUseDeleteUserConnection.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseCreateApiKeyConnection.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
  });

  it("renders available provider in the list", () => {
    render(<ConnectionsPage />, { wrapper });

    expect(screen.getByText("Daytona")).toBeInTheDocument();
    expect(screen.getByText("Cloud development environment")).toBeInTheDocument();
  });

  it("renders instructions_markdown via InlineStreamdownMessage when dialog opens", async () => {
    render(<ConnectionsPage />, { wrapper });

    // Click Connect button to open the API key dialog
    const connectButton = screen.getByRole("button", { name: /Connect/i });
    fireEvent.click(connectButton);

    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    // Verify InlineStreamdownMessage is used with the raw markdown instructions
    const markdownEl = screen.getByTestId("inline-streamdown-message");
    expect(markdownEl).toBeInTheDocument();
    // Raw markdown is passed as children (component handles rendering)
    expect(markdownEl).toHaveTextContent("[Daytona Dashboard](https://app.daytona.io)");
    expect(markdownEl).toHaveTextContent("**API Keys**");
    expect(markdownEl).toHaveTextContent("**Create New API Key**");
  });

  it("passes styling classes to InlineStreamdownMessage", async () => {
    render(<ConnectionsPage />, { wrapper });

    const connectButton = screen.getByRole("button", { name: /Connect/i });
    fireEvent.click(connectButton);

    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    const markdownEl = screen.getByTestId("inline-streamdown-message");
    expect(markdownEl.className).toContain("text-sm");
    expect(markdownEl.className).toContain("text-muted-foreground");
  });

  it("does not render InlineStreamdownMessage when dialog is closed", () => {
    render(<ConnectionsPage />, { wrapper });

    expect(screen.queryByTestId("inline-streamdown-message")).not.toBeInTheDocument();
  });

  it("renders form fields alongside markdown instructions", async () => {
    render(<ConnectionsPage />, { wrapper });

    const connectButton = screen.getByRole("button", { name: /Connect/i });
    fireEvent.click(connectButton);

    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    // Both markdown instructions and form fields should be present
    expect(screen.getByTestId("inline-streamdown-message")).toBeInTheDocument();
    expect(screen.getByLabelText("API Key")).toBeInTheDocument();
  });

  it("renders Connect button with provider name in dialog title", async () => {
    render(<ConnectionsPage />, { wrapper });

    const connectButton = screen.getByRole("button", { name: /Connect/i });
    fireEvent.click(connectButton);

    await waitFor(() => {
      expect(screen.getByText("Connect Daytona")).toBeInTheDocument();
    });
  });
});
