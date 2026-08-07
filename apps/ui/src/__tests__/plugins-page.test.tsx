import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { CatalogEntryRow } from "@/app/(main)/plugins/page";

const mockInstallPlugin = jest.fn();

jest.mock("@/lib/api/plugins", () => ({
  ...jest.requireActual("@/lib/api/plugins"),
  installPlugin: (...args: unknown[]) => mockInstallPlugin(...args),
}));

describe("Plugins page catalog install", () => {
  it("renders a rejected installation inline without an unhandled mutation promise", async () => {
    mockInstallPlugin.mockRejectedValueOnce(new Error("archive rejected"));
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    render(
      <CatalogEntryRow
        marketplaceId="marketplace-1"
        entry={{
          name: "resend",
          display_name: "Resend",
          description: "Send transactional email",
          version: "0.1.0",
          author: "Everruns",
          category: "Productivity",
          installed: false,
        }}
      />,
      { wrapper },
    );

    fireEvent.click(screen.getByRole("button", { name: "Install" }));

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("archive rejected");
    });
  });
});
