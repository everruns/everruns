/**
 * The theme switch has three jobs: honor an explicit choice, follow the OS when
 * the user has not chosen, and keep `.dark` on <html> in sync with both.
 */
import { render, screen, act, fireEvent } from "@testing-library/react";
import { ThemeProvider, useTheme } from "@/providers/theme-provider";
import { parseThemeMode, persistThemeMode, THEME_COOKIE, type ThemeMode } from "@/lib/theme";

type MediaListener = () => void;

function mockPrefersDark(matches: boolean) {
  const listeners = new Set<MediaListener>();
  const query = {
    matches,
    addEventListener: (_: string, listener: MediaListener) => listeners.add(listener),
    removeEventListener: (_: string, listener: MediaListener) => listeners.delete(listener),
  };
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: jest.fn().mockReturnValue(query),
  });
  return {
    setMatches(next: boolean) {
      query.matches = next;
      act(() => listeners.forEach((listener) => listener()));
    },
  };
}

function Probe() {
  const { mode, theme, setMode } = useTheme();
  return (
    <div>
      <span data-testid="mode">{mode}</span>
      <span data-testid="theme">{theme}</span>
      <button onClick={() => setMode("dark")}>go dark</button>
      <button onClick={() => setMode("system")}>go system</button>
    </div>
  );
}

beforeEach(() => {
  document.documentElement.classList.remove("dark");
  document.cookie = `${THEME_COOKIE}=; path=/; max-age=0`;
});

describe("parseThemeMode", () => {
  it("keeps the three known modes and falls back to system", () => {
    expect(parseThemeMode("light")).toBe("light");
    expect(parseThemeMode("dark")).toBe("dark");
    expect(parseThemeMode("system")).toBe("system");
    expect(parseThemeMode(undefined)).toBe("system");
    expect(parseThemeMode("solarized")).toBe("system");
  });
});

describe("persistThemeMode", () => {
  it("writes only a known mode, so no caller can append cookie attributes", () => {
    persistThemeMode("bogus; path=/; domain=evil.example" as ThemeMode);
    expect(document.cookie).toContain(`${THEME_COOKIE}=system`);
    expect(document.cookie).not.toContain("evil.example");
  });
});

describe("ThemeProvider", () => {
  it("applies an explicit dark mode from the server-rendered cookie", () => {
    mockPrefersDark(false);
    render(
      <ThemeProvider initialMode="dark">
        <Probe />
      </ThemeProvider>,
    );

    expect(screen.getByTestId("theme")).toHaveTextContent("dark");
    expect(document.documentElement).toHaveClass("dark");
  });

  it("keeps an explicit light mode on a dark-preferring OS", () => {
    mockPrefersDark(true);
    render(
      <ThemeProvider initialMode="light">
        <Probe />
      </ThemeProvider>,
    );

    expect(screen.getByTestId("theme")).toHaveTextContent("light");
    expect(document.documentElement).not.toHaveClass("dark");
  });

  it("resolves system mode from the OS preference", () => {
    mockPrefersDark(true);
    render(
      <ThemeProvider initialMode="system">
        <Probe />
      </ThemeProvider>,
    );

    expect(screen.getByTestId("theme")).toHaveTextContent("dark");
    expect(document.documentElement).toHaveClass("dark");
  });

  it("follows a live OS change while in system mode, but not after an explicit choice", () => {
    const media = mockPrefersDark(false);
    render(
      <ThemeProvider initialMode="system">
        <Probe />
      </ThemeProvider>,
    );

    media.setMatches(true);
    expect(document.documentElement).toHaveClass("dark");

    fireEvent.click(screen.getByText("go dark"));
    media.setMatches(false);
    expect(screen.getByTestId("theme")).toHaveTextContent("dark");
    expect(document.documentElement).toHaveClass("dark");
  });

  it("persists the chosen mode to the cookie so the next server render matches", () => {
    mockPrefersDark(false);
    render(
      <ThemeProvider initialMode="system">
        <Probe />
      </ThemeProvider>,
    );

    fireEvent.click(screen.getByText("go dark"));
    expect(screen.getByTestId("mode")).toHaveTextContent("dark");
    expect(document.cookie).toContain(`${THEME_COOKIE}=dark`);

    fireEvent.click(screen.getByText("go system"));
    expect(document.cookie).toContain(`${THEME_COOKIE}=system`);
  });
});
