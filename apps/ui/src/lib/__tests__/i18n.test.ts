import {
  detectBrowserLocale,
  formatEditCount,
  formatImageCount,
  formatMessage,
  getSupportedLocale,
} from "@/lib/i18n";

describe("i18n locale helpers", () => {
  it("detects Ukrainian browser locale from navigator.languages", () => {
    expect(detectBrowserLocale({ languages: ["uk-UA", "en-US"], language: "en-US" })).toBe("uk-UA");
    expect(getSupportedLocale("uk-UA")).toBe("uk");
  });

  it("falls back to English UI copy for unsupported locales", () => {
    expect(getSupportedLocale("fr-FR")).toBe("en");
    expect(formatMessage("en", "no_messages_yet")).toBe("No messages yet");
  });

  it("formats Ukrainian strings with interpolation", () => {
    expect(formatMessage("uk", "iteration", { value: 3 })).toBe("Ітерація 3");
  });

  it("formats image counts with Ukrainian plural forms", () => {
    expect(formatImageCount("uk", 1)).toBe("1 зображення");
    expect(formatImageCount("uk", 2)).toBe("2 зображення");
    expect(formatImageCount("uk", 5)).toBe("5 зображень");
  });

  it("formats edit counts with Ukrainian plural forms", () => {
    expect(formatEditCount("uk", 1)).toBe("1 зміна");
    expect(formatEditCount("uk", 3)).toBe("3 зміни");
    expect(formatEditCount("uk", 11)).toBe("11 змін");
  });
});
