import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import React from "react";
import { AddProviderDialog } from "@/app/(main)/settings/providers/provider-dialogs";

// Render the real credential form, but swap the base-ui Select for a native
// <select> so the provider-type picker is drivable in jsdom (mirrors the mock
// used by knowledge-indexes-page.test.tsx).
jest.mock("@/components/ui/select", () => {
  function collectOptions(node: React.ReactNode): React.ReactNode[] {
    const options: React.ReactNode[] = [];
    React.Children.forEach(node, (child) => {
      if (!React.isValidElement(child)) return;
      if ((child.type as { isSelectItem?: boolean }).isSelectItem) {
        const props = child.props as { value: string; children: React.ReactNode };
        options.push(
          <option key={props.value} value={props.value}>
            {props.value}
          </option>,
        );
        return;
      }
      options.push(...collectOptions((child.props as { children?: React.ReactNode }).children));
    });
    return options;
  }

  function Select({
    value,
    onValueChange,
    children,
  }: {
    value: string;
    onValueChange: (value: string) => void;
    children: React.ReactNode;
  }) {
    return (
      <select
        aria-label="Provider Type"
        value={value}
        onChange={(event) => onValueChange(event.target.value)}
      >
        {collectOptions(children)}
      </select>
    );
  }

  function SelectItem(_: { value: string; children: React.ReactNode }) {
    return null;
  }
  SelectItem.isSelectItem = true;

  return {
    Select,
    SelectContent: ({ children }: { children: React.ReactNode }) => <>{children}</>,
    SelectItem,
    SelectTrigger: ({ children }: { children: React.ReactNode }) => <>{children}</>,
    SelectValue: () => null,
  };
});

const mockCreateProvider = jest.fn();
const mockUseProvidersConfig = jest.fn();

jest.mock("@/hooks/use-providers", () => ({
  useCreateProvider: () => ({ mutateAsync: mockCreateProvider, isPending: false }),
  useUpdateProvider: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useProvidersConfig: () => mockUseProvidersConfig(),
}));

jest.mock("@/lib/api/providers", () => ({
  providerSupportsOAuth: (driver: string) => driver === "openrouter",
  providerOAuthAuthorizeUrl: (id: string) => `/api/v1/providers/${id}/oauth/authorize`,
}));

const apiKeyDriver = (driver: string, supportsOAuth: boolean) => ({
  driver,
  supports_oauth: supportsOAuth,
  credential_schema: {
    fields: [
      { name: "api_key", label: "API Key", field_type: "password", required: !supportsOAuth },
    ],
    instructions_markdown: "",
  },
});

describe("AddProviderDialog OAuth connect", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseProvidersConfig.mockReturnValue({
      data: {
        policies: {},
        drivers: [apiKeyDriver("openai", false), apiKeyDriver("openrouter", true)],
      },
      isLoading: false,
      error: null,
    });
  });

  it("offers a Connect button for OAuth drivers and not for key-only drivers", async () => {
    render(<AddProviderDialog open onOpenChange={() => {}} />);

    // Default driver (openai) is key-only: no Connect button.
    expect(screen.queryByRole("button", { name: /Connect with/i })).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Provider Type"), { target: { value: "openrouter" } });

    expect(
      await screen.findByRole("button", { name: "Connect with OpenRouter" }),
    ).toBeInTheDocument();
  });

  it("creates the provider without a key before starting the OAuth flow", async () => {
    mockCreateProvider.mockResolvedValue({ id: "provider-new" });

    render(<AddProviderDialog open onOpenChange={() => {}} />);

    fireEvent.change(screen.getByLabelText("Provider Type"), { target: { value: "openrouter" } });
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "My OpenRouter" } });

    fireEvent.click(await screen.findByRole("button", { name: "Connect with OpenRouter" }));

    // The Connect button creates the provider with no credentials; the returned
    // id is what the authorize redirect (asserted via providerOAuthAuthorizeUrl
    // elsewhere) binds its OAuth state to.
    await waitFor(() => {
      expect(mockCreateProvider).toHaveBeenCalledWith({
        name: "My OpenRouter",
        provider_type: "openrouter",
        base_url: undefined,
        credentials: undefined,
      });
    });
  });
});
