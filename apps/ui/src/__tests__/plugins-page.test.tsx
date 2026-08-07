import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement, ReactNode } from "react";
import { CatalogEntryRow } from "@/app/(main)/plugins/page";
import { ApiError } from "@/lib/api/client";
import type { InstalledPlugin, MarketplaceCatalogEntry } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";

const mockInstallPlugin = jest.fn();
const mockGetInstalledPlugins = jest.fn();

jest.mock("@/lib/api/plugins", () => ({
  ...jest.requireActual("@/lib/api/plugins"),
  installPlugin: (...args: unknown[]) => mockInstallPlugin(...args),
  getInstalledPlugins: (...args: unknown[]) => mockGetInstalledPlugins(...args),
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org-1" }, isLoading: false }),
}));

const entry: MarketplaceCatalogEntry = {
  name: "resend",
  display_name: "Resend",
  description: "Send transactional email",
  version: "1.0.0",
  author: "Everruns",
  category: "Productivity",
  installed: false,
};

function installedPlugin(overrides: Partial<InstalledPlugin> = {}): InstalledPlugin {
  return {
    id: "plugin-1",
    name: "resend",
    display_name: "Resend",
    description: "Send transactional email",
    version: "1.0.0",
    pinned_sha: "abc123",
    marketplace: "official",
    capability_ref: "plugin:resend",
    status: "active",
    warnings: [],
    update_available: false,
    created_at: "2026-08-06T00:00:00Z",
    updated_at: "2026-08-06T00:00:00Z",
    ...overrides,
  };
}

describe("Plugins page catalog install", () => {
  let queryClient: QueryClient;
  let wrapper: ({ children }: { children: ReactNode }) => ReactElement;

  beforeEach(() => {
    jest.clearAllMocks();
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });
    wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  });

  function renderRow(installedPlugins: InstalledPlugin[] = [], catalogEntry = entry) {
    return render(
      <CatalogEntryRow
        marketplaceId="marketplace-1"
        marketplaceName="official"
        installedPlugins={installedPlugins}
        entry={catalogEntry}
      />,
      { wrapper },
    );
  }

  it("reconciles the installed list and open catalog immediately after success", async () => {
    const installed = installedPlugin();
    const installedKey = [...queryKeys.installedPlugins.list(), "org-1"];
    const catalogKey = [...queryKeys.pluginMarketplaces.catalog("marketplace-1"), "org-1"];
    const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");
    queryClient.setQueryData(installedKey, []);
    queryClient.setQueryData(catalogKey, [entry]);
    mockInstallPlugin.mockResolvedValueOnce(installed);

    renderRow();
    fireEvent.click(screen.getByRole("button", { name: "Install" }));

    await waitFor(() => expect(screen.getByRole("button", { name: "Installed" })).toBeDisabled());
    expect(queryClient.getQueryData(installedKey)).toEqual([installed]);
    expect(queryClient.getQueryData(catalogKey)).toEqual([{ ...entry, installed: true }]);
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: installedKey, exact: true });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: catalogKey, exact: true });
  });

  it("blocks a second click synchronously while installation is pending", async () => {
    mockInstallPlugin.mockReturnValue(new Promise(() => undefined));
    renderRow();

    const button = screen.getByRole("button", { name: "Install" });
    act(() => {
      button.click();
      button.click();
    });

    await waitFor(() => expect(mockInstallPlugin).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("button", { name: "Installing..." })).toBeDisabled();
  });

  it("treats an already-installed race as success after inventory reconciliation", async () => {
    const installed = installedPlugin();
    mockInstallPlugin.mockRejectedValueOnce(
      new ApiError(409, "Conflict", "Plugin 'resend' is already installed"),
    );
    mockGetInstalledPlugins.mockResolvedValueOnce([installed]);

    renderRow();
    fireEvent.click(screen.getByRole("button", { name: "Install" }));

    await waitFor(() => expect(screen.getByRole("button", { name: "Installed" })).toBeDisabled());
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("preserves a real installation failure as inline feedback", async () => {
    mockInstallPlugin.mockRejectedValueOnce(new Error("archive rejected"));
    renderRow();

    fireEvent.click(screen.getByRole("button", { name: "Install" }));

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("archive rejected");
    });
    expect(screen.getByRole("button", { name: "Install" })).toBeEnabled();
  });

  it("retains the installed state when the catalog row is reopened", async () => {
    const installed = installedPlugin();
    const installedKey = [...queryKeys.installedPlugins.list(), "org-1"];
    queryClient.setQueryData(installedKey, []);
    mockInstallPlugin.mockResolvedValueOnce(installed);

    const first = renderRow();
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Installed" })).toBeDisabled());
    first.unmount();

    renderRow(queryClient.getQueryData<InstalledPlugin[]>(installedKey) ?? []);
    expect(screen.getByRole("button", { name: "Installed" })).toBeDisabled();
  });

  it("distinguishes an available upgrade from the installed catalog version", () => {
    renderRow([installedPlugin({ version: "0.9.0", update_available: true })]);

    expect(screen.getByRole("button", { name: "Upgrade available" })).toBeDisabled();
    expect(
      screen.getByText("Version 0.9.0 is installed. Update it from Installed Plugins."),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: "Install" })).not.toBeInTheDocument();
  });

  it("preserves a conflict when the same plugin name is installed from another marketplace", async () => {
    mockInstallPlugin.mockRejectedValueOnce(
      new ApiError(409, "Conflict", "Plugin 'resend' is already installed"),
    );
    mockGetInstalledPlugins.mockResolvedValueOnce([
      installedPlugin({ marketplace: "private-marketplace" }),
    ]);
    renderRow();

    fireEvent.click(screen.getByRole("button", { name: "Install" }));

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("already installed");
    });
  });
});
