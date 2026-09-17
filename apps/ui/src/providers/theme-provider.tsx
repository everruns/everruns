"use client";

/**
 * Decisions:
 * - The provider owns the mode; the `.dark` class on <html> is a side effect of
 *   it, so every consumer reads one source of truth.
 * - The resolved theme is derived during render, not mirrored into state — only
 *   the OS preference is state, because only it changes from outside React.
 * - "system" keeps listening to the OS after mount, so an OS switch flips the
 *   app without a reload.
 * - Outside the app shell (tests, isolated stories) the fallback context is a
 *   no-op light theme rather than a thrown error, matching LocaleProvider.
 */

import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  applyTheme,
  persistThemeMode,
  prefersDark,
  type ResolvedTheme,
  type ThemeMode,
} from "@/lib/theme";

interface ThemeContextValue {
  /** What the user chose, including "system". */
  mode: ThemeMode;
  /** What is actually rendered right now. */
  theme: ResolvedTheme;
  setMode: (mode: ThemeMode) => void;
}

const fallbackThemeContext: ThemeContextValue = {
  mode: "system",
  theme: "light",
  setMode: () => {},
};

const ThemeContext = createContext<ThemeContextValue>(fallbackThemeContext);

export function ThemeProvider({
  initialMode,
  children,
}: {
  initialMode: ThemeMode;
  children: ReactNode;
}) {
  const [mode, setModeState] = useState<ThemeMode>(initialMode);
  // On the server this reads light; THEME_SCRIPT has already corrected the
  // class by the time this component hydrates, and nothing in the initial
  // markup depends on the value (the theme menu mounts only when opened).
  const [systemTheme, setSystemTheme] = useState<ResolvedTheme>(() =>
    prefersDark() ? "dark" : "light",
  );

  const theme: ResolvedTheme = mode === "system" ? systemTheme : mode;

  useEffect(() => {
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setSystemTheme(query.matches ? "dark" : "light");
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  const setMode = useCallback((next: ThemeMode) => {
    setModeState(next);
    persistThemeMode(next);
  }, []);

  const value = useMemo<ThemeContextValue>(
    () => ({ mode, theme, setMode }),
    [mode, theme, setMode],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme() {
  return useContext(ThemeContext);
}
