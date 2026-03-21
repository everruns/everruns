"use client";

/**
 * Decisions:
 * - Detect browser locale once on mount and reuse it across the app.
 * - Expose both UI locale and backend locale so browser-driven overrides stay explicit.
 */

import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  detectBrowserLocale,
  formatMessage,
  getSupportedLocale,
  type MessageKey,
  type SupportedLocale,
} from "@/lib/i18n";

interface LocaleContextValue {
  locale: SupportedLocale;
  backendLocale: string;
  t: (key: MessageKey, values?: Record<string, string | number>) => string;
}

const LocaleContext = createContext<LocaleContextValue | null>(null);

function getInitialBrowserLocale(): string {
  if (typeof window === "undefined") {
    return "en-US";
  }

  return detectBrowserLocale(window.navigator);
}

export function LocaleProvider({ children }: { children: ReactNode }) {
  const [backendLocale, setBackendLocale] = useState(getInitialBrowserLocale);
  const [locale, setLocale] = useState<SupportedLocale>(() =>
    getSupportedLocale(getInitialBrowserLocale()),
  );

  useEffect(() => {
    const browserLocale = detectBrowserLocale(window.navigator);
    setBackendLocale(browserLocale);
    setLocale(getSupportedLocale(browserLocale));
    document.documentElement.lang = browserLocale;
  }, []);

  const value = useMemo<LocaleContextValue>(
    () => ({
      locale,
      backendLocale,
      t: (key, values) => formatMessage(locale, key, values),
    }),
    [backendLocale, locale],
  );

  return <LocaleContext.Provider value={value}>{children}</LocaleContext.Provider>;
}

export function useLocale() {
  const context = useContext(LocaleContext);
  if (!context) {
    throw new Error("useLocale must be used within a LocaleProvider");
  }
  return context;
}
