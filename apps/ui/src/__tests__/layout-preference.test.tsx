import { act, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import {
  parseAppLayout,
  parseClassicChat,
  useClassicChatPreference,
  useLayoutPreference,
} from "@/hooks/use-layout-preference";
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
  it("treats anything but an explicit chat first choice as classic", () => {
    expect(parseAppLayout("classic")).toBe("classic");
    expect(parseAppLayout("chat_first")).toBe("chat_first");
    expect(parseAppLayout(undefined)).toBe("classic");
    expect(parseAppLayout(42)).toBe("classic");
  });

  it("defaults to classic when the person never chose", async () => {
    getUserPreference.mockResolvedValue(null);
    const { result } = renderHook(() => useLayoutPreference(), { wrapper });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.layout).toBe("classic");
    expect(window.localStorage.getItem("everruns.ui.layout")).toBeNull();
  });

  it("keeps a stored chat first choice over the classic default", async () => {
    getUserPreference.mockResolvedValue({ key: "ui.layout", value: "chat_first", updated_at: "" });
    const { result } = renderHook(() => useLayoutPreference(), { wrapper });
    await waitFor(() => expect(result.current.layout).toBe("chat_first"));
    expect(window.localStorage.getItem("everruns.ui.layout")).toBe("chat_first");
  });

  it("lets the server choice win over this device's last layout", async () => {
    window.localStorage.setItem("everruns.ui.layout", "classic");
    getUserPreference.mockResolvedValue({ key: "ui.layout", value: "chat_first", updated_at: "" });
    const { result } = renderHook(() => useLayoutPreference(), { wrapper });
    await waitFor(() => expect(result.current.layout).toBe("chat_first"));
  });

  it("reads the stored choice and mirrors it on the device", async () => {
    getUserPreference.mockResolvedValue({ key: "ui.layout", value: "classic", updated_at: "" });
    const { result } = renderHook(() => useLayoutPreference(), { wrapper });
    // Classic is also the default, so wait for the mirror that proves the read.
    await waitFor(() => expect(window.localStorage.getItem("everruns.ui.layout")).toBe("classic"));
    expect(result.current.layout).toBe("classic");
  });

  it("uses the device's last layout before the server answers", () => {
    window.localStorage.setItem("everruns.ui.layout", "chat_first");
    getUserPreference.mockReturnValue(new Promise(() => {}));
    const { result } = renderHook(() => useLayoutPreference(), { wrapper });
    expect(result.current.layout).toBe("chat_first");
    expect(result.current.ready).toBe(true);
  });

  it("switches immediately and saves the choice from Profile", async () => {
    getUserPreference.mockResolvedValue(null);
    render(<LayoutSettings />, { wrapper });
    const chatFirst = await screen.findByRole("radio", { name: /Chat first/ });
    expect(screen.getByRole("radio", { name: /Classic/ })).toHaveAttribute("aria-checked", "true");
    await act(async () => {
      fireEvent.click(chatFirst);
    });
    await waitFor(() => expect(chatFirst).toHaveAttribute("aria-checked", "true"));
    expect(setUserPreference).toHaveBeenCalledWith("ui.layout", "chat_first");
  });

  it("keeps only known Classic chat options, full screen otherwise", () => {
    expect(parseClassicChat("floating")).toBe("floating");
    expect(parseClassicChat("docked")).toBe("docked");
    expect(parseClassicChat("full_screen")).toBe("full_screen");
    expect(parseClassicChat(undefined)).toBe("full_screen");
    expect(parseClassicChat("sidebar")).toBe("full_screen");
  });

  it("reads Classic chat from its own server-side preference", async () => {
    getUserPreference.mockImplementation(async (key: string) =>
      key === "ui.classic_chat" ? { key, value: "docked", updated_at: "" } : null,
    );
    const { result } = renderHook(() => useClassicChatPreference(), { wrapper });
    await waitFor(() => expect(result.current.classicChat).toBe("docked"));
    expect(window.localStorage.getItem("everruns.ui.classic_chat")).toBe("docked");
  });

  it("saves floating Chat from Profile and switches the layout to Classic", async () => {
    getUserPreference.mockImplementation(async (key: string) =>
      key === "ui.layout" ? { key, value: "chat_first", updated_at: "" } : null,
    );
    render(<LayoutSettings />, { wrapper });
    const group = await screen.findByRole("radiogroup", { name: "Chat in Classic" });
    // Chat first is stored, so the Classic options say when they apply.
    expect(await screen.findByText("Applies when the layout is Classic.")).toBeInTheDocument();
    const floating = screen.getByRole("radio", { name: /^Floating/ });
    expect(group).toContainElement(floating);
    expect(screen.getByRole("radio", { name: /^Full screen/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await act(async () => {
      fireEvent.click(floating);
    });
    await waitFor(() => expect(floating).toHaveAttribute("aria-checked", "true"));
    expect(setUserPreference).toHaveBeenCalledWith("ui.classic_chat", "floating");
    expect(setUserPreference).toHaveBeenCalledWith("ui.layout", "classic");
  });
});
