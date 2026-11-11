import { act, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { parseAppLayout, useLayoutPreference } from "@/hooks/use-layout-preference";
import { LayoutSettings } from "@/components/settings/layout-settings";

jest.mock("@/providers/locale-provider", () => ({
  useUiTranslations: () => (key: string) => key,
}));

const getUserPreference = jest.fn();
const setUserPreference = jest.fn();
jest.mock("@/lib/api/user-preferences", () => ({
  getUserPreference: (key: string) => getUserPreference(key),
  setUserPreference: (key: string, value: unknown) => setUserPreference(key, value),
}));

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

beforeEach(() => {
  window.localStorage.clear();
  getUserPreference.mockReset();
  setUserPreference.mockReset();
  setUserPreference.mockImplementation(async (key: string, value: unknown) => ({
    key,
    value,
    updated_at: "2026-11-11T00:00:00Z",
  }));
});

describe("layout preference", () => {
  it("treats anything but classic as chat first", () => {
    expect(parseAppLayout("classic")).toBe("classic");
    expect(parseAppLayout("chat_first")).toBe("chat_first");
    expect(parseAppLayout(undefined)).toBe("chat_first");
    expect(parseAppLayout(42)).toBe("chat_first");
  });

  it("defaults to chat first when the person never chose", async () => {
    getUserPreference.mockResolvedValue(null);
    const { result } = renderHook(() => useLayoutPreference(), { wrapper });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.layout).toBe("chat_first");
  });

  it("reads the stored choice and mirrors it on the device", async () => {
    getUserPreference.mockResolvedValue({ key: "ui.layout", value: "classic", updated_at: "" });
    const { result } = renderHook(() => useLayoutPreference(), { wrapper });
    await waitFor(() => expect(result.current.layout).toBe("classic"));
    expect(window.localStorage.getItem("everruns.ui.layout")).toBe("classic");
  });

  it("uses the device's last layout before the server answers", () => {
    window.localStorage.setItem("everruns.ui.layout", "classic");
    getUserPreference.mockReturnValue(new Promise(() => {}));
    const { result } = renderHook(() => useLayoutPreference(), { wrapper });
    expect(result.current.layout).toBe("classic");
    expect(result.current.ready).toBe(true);
  });

  it("switches immediately and saves the choice from Profile", async () => {
    getUserPreference.mockResolvedValue(null);
    render(<LayoutSettings />, { wrapper });
    const classic = await screen.findByRole("radio", { name: /Classic/ });
    expect(screen.getByRole("radio", { name: /Chat first/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await act(async () => {
      fireEvent.click(classic);
    });
    expect(classic).toHaveAttribute("aria-checked", "true");
    expect(setUserPreference).toHaveBeenCalledWith("ui.layout", "classic");
  });
});
