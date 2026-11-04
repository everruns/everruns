import {
  detectInterfaceLocale,
  getSupportedLocale,
  isSupportedLocale,
} from "@/lib/locale-registry";
import { formatCountLabel, formatShortDate } from "@/lib/formatting";

describe("interface locale negotiation", () => {
  it.each([
    ["fr-CA", "fr"],
    ["de_AT", "de"],
    ["es-MX", "es"],
    ["zh-Hans-CN", "zh-CN"],
    ["en-GB", "en-GB"],
    ["uk-UA", "uk"],
    ["en-US", "en"],
    ["ja-JP", "en"],
    ["constructor", "en"],
    ["__proto__", "en"],
    ["zh-Hant-TW", "en"],
  ])("resolves %s to %s", (input, expected) => expect(getSupportedLocale(input)).toBe(expected));

  it("uses the next supported browser preference without changing its region", () => {
    expect(detectInterfaceLocale({ languages: ["ja-JP", "fr-CA"], language: "ja-JP" })).toBe(
      "fr-CA",
    );
    expect(detectInterfaceLocale({ languages: ["zh-TW", "de-DE"], language: "zh-TW" })).toBe(
      "de-DE",
    );
    expect(detectInterfaceLocale({ languages: [], language: "en-GB" })).toBe("en-GB");
    expect(detectInterfaceLocale({ languages: ["xx"], language: "xx" })).toBe("en");
  });

  it("accepts only explicit locale preferences, including British English", () => {
    expect(isSupportedLocale("en-GB")).toBe(true);
    expect(isSupportedLocale("de")).toBe(true);
    expect(isSupportedLocale("constructor")).toBe(false);
    expect(isSupportedLocale("__proto__")).toBe(false);
  });
});

describe("regional formatting", () => {
  it.each([
    ["fr", 1, "1 agent"],
    ["fr", 2, "2 agents"],
    ["de", 2, "2 Agenten"],
    ["es", 2, "2 agentes"],
    ["zh-CN", 2, "2 智能体"],
    ["uk", 22, "22 агенти"],
  ])("formats %s product counters", (locale, count, expected) => {
    expect(formatCountLabel(count as number, "agent", undefined, locale as string)).toBe(expected);
  });

  it("keeps British day/month ordering", () => {
    expect(formatShortDate("2026-06-15T12:00:00Z", "en-GB")).toBe("15/06/2026");
  });
});
