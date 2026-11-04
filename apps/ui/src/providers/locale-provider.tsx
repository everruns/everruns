"use client";

/**
 * Decisions:
 * - Detect browser locale once on mount and reuse it across the app.
 * - Expose both UI locale and backend locale so browser-driven overrides stay explicit.
 * - Fall back to English when components render outside the app shell (tests, isolated stories).
 */

import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  useCallback,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import {
  detectBrowserLocale,
  formatMessage,
  getSupportedLocale,
  type MessageKey,
  type SupportedLocale,
} from "@/lib/i18n";
import { uiMessagesByLocale } from "@/lib/ui-messages";
import { detectInterfaceLocale, isSupportedLocale } from "@/lib/locale-registry";

interface LocaleContextValue {
  locale: SupportedLocale;
  displayLocale: string;
  preference: "system" | SupportedLocale;
  setPreference: (value: "system" | SupportedLocale) => void;
  backendLocale: string;
  t: (key: MessageKey, values?: Record<string, string | number>) => string;
}

const fallbackLocaleContext: LocaleContextValue = {
  locale: "en",
  displayLocale: "en",
  preference: "system",
  setPreference: () => {},
  backendLocale: "en-US",
  t: (key, values) => formatMessage("en", key, values),
};

const LocaleContext = createContext<LocaleContextValue>(fallbackLocaleContext);

const PREFERENCE_KEY = "everruns.ui-language";
function readPreference(): "system" | SupportedLocale {
  try {
    const value = localStorage.getItem(PREFERENCE_KEY);
    return isSupportedLocale(value) ? value : "system";
  } catch {
    return "system";
  }
}
function subscribePreference(callback: () => void) {
  window.addEventListener("storage", callback);
  window.addEventListener("everruns:language", callback);
  return () => {
    window.removeEventListener("storage", callback);
    window.removeEventListener("everruns:language", callback);
  };
}
function setPreference(value: "system" | SupportedLocale) {
  try {
    localStorage.setItem(PREFERENCE_KEY, value);
  } catch {
    /* Private browsing may disable storage. */
  }
  window.dispatchEvent(new Event("everruns:language"));
}

export function LocaleProvider({ children }: { children: ReactNode }) {
  const preference = useSyncExternalStore(
    subscribePreference,
    readPreference,
    () => "system" as const,
  );
  const [backendLocale, setBackendLocale] = useState("en-US");
  const [browserDisplayLocale, setBrowserDisplayLocale] = useState("en-US");
  const displayLocale = preference === "system" ? browserDisplayLocale : preference;
  const locale = getSupportedLocale(displayLocale);

  useEffect(() => {
    const browserLocale = detectBrowserLocale(window.navigator);
    setBackendLocale(browserLocale);
    setBrowserDisplayLocale(detectInterfaceLocale(window.navigator));
  }, []);

  useEffect(() => {
    document.documentElement.lang = locale === "zh-CN" ? "zh-Hans" : displayLocale;
  }, [displayLocale, locale]);

  const value = useMemo<LocaleContextValue>(
    () => ({
      locale,
      displayLocale,
      preference,
      setPreference,
      backendLocale,
      t: (key, values) => formatMessage(locale, key, values),
    }),
    [backendLocale, locale, displayLocale, preference],
  );

  return <LocaleContext.Provider value={value}>{children}</LocaleContext.Provider>;
}

export function useLocale() {
  return useContext(LocaleContext);
}

/** Product-owned source messages act as gettext-style IDs; never pass user content. */
export function useUiTranslations() {
  const { locale } = useLocale();
  return useCallback(
    (message: string, values?: Record<string, unknown>): string => {
      const template = uiMessagesByLocale[locale]?.[message] ?? message;
      return values
        ? template.replace(/\{(\w+)\}/g, (token, key: string) =>
            key in values ? String(values[key]) : token,
          )
        : template;
    },
    [locale],
  );
}
