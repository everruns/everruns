/** Interface locales are independent of execution locales and retain regional variants. */
export const interfaceLocales = {
  en: "English",
  "en-GB": "English (United Kingdom)",
  uk: "Українська",
  fr: "Français",
  de: "Deutsch",
  es: "Español",
  "zh-CN": "中文（简体）",
} as const;

export type SupportedLocale = keyof typeof interfaceLocales;

export function isSupportedLocale(value: unknown): value is SupportedLocale {
  return typeof value === "string" && Object.hasOwn(interfaceLocales, value);
}

function matchInterfaceLocale(raw: string): SupportedLocale | undefined {
  const tag = raw.trim().replace(/_/g, "-").toLowerCase();
  if (tag === "en-gb" || tag.startsWith("en-gb-")) return "en-GB";
  // Traditional Chinese has no catalog yet; try the next browser preference.
  if (/^zh-(?:hant|tw|hk|mo)(?:-|$)/.test(tag)) return undefined;
  const language = tag.split("-")[0];
  if (language === "zh") return "zh-CN";
  return isSupportedLocale(language) ? language : undefined;
}

export function getSupportedLocale(locale: string): SupportedLocale {
  return matchInterfaceLocale(locale) ?? "en";
}

/** Select the first supported browser preference while preserving its formatting region. */
export function detectInterfaceLocale(
  navigatorLike?: Pick<Navigator, "languages" | "language">,
): string {
  const candidates = [...(navigatorLike?.languages ?? []), navigatorLike?.language];
  for (const candidate of candidates) {
    if (candidate && matchInterfaceLocale(candidate)) return candidate.trim().replace(/_/g, "-");
  }
  return "en";
}
