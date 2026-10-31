import { semanticMessages } from "@/lib/semantic-messages";
import { type SupportedLocale, type MessageKey, englishMessages } from "@/lib/i18n";
import { uiMessages, uiMessagesByLocale } from "@/lib/ui-messages";

const tokens = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();

describe("Ukrainian product catalog", () => {
  it("does not treat object prototype names as translations", () => {
    expect(uiMessages["constructor"]).toBeUndefined();
    expect(uiMessages["__proto__"]).toBeUndefined();
  });
  it("contains complete translations without draft markers", () => {
    const invalid = Object.entries(uiMessages).filter(
      ([, text]) => !text.trim() || /⟦\d+⟧/.test(text),
    );
    expect(invalid).toEqual([]);
  });
  it("preserves interpolation values in every translated template", () => {
    const invalid = Object.entries(uiMessages).filter(
      ([source, text]) => JSON.stringify(tokens(source)) !== JSON.stringify(tokens(text)),
    );
    expect(invalid).toEqual([]);
  });
});

describe.each(["fr", "de", "es", "zh-CN"])("%s product catalog", (locale) => {
  const catalog = uiMessagesByLocale[locale];
  it("covers existing product and semantic messages", () => {
    const required = new Set([...Object.keys(uiMessages), ...Object.values(englishMessages)]);
    const missing = [...required].filter((source) => !Object.hasOwn(catalog, source));
    expect(missing).toEqual([]);
  });
  it("preserves interpolation and safely rejects prototype keys", () => {
    const invalid = Object.entries(catalog).filter(
      ([source, text]) =>
        !text.trim() ||
        /⟦\d+⟧|ZXQPH/i.test(text) ||
        JSON.stringify(tokens(source)) !== JSON.stringify(tokens(text)),
    );
    expect(invalid).toEqual([]);
    for (const [key, text] of Object.entries(semanticMessages[locale as SupportedLocale] ?? {})) {
      expect(text.trim()).not.toBe("");
      expect(tokens(text)).toEqual(tokens(englishMessages[key as MessageKey]));
    }
    expect(catalog["constructor"]).toBeUndefined();
    expect(catalog["__proto__"]).toBeUndefined();
  });
  it("preserves filenames in translated product copy", () => {
    for (const [source, text] of Object.entries(catalog)) {
      for (const filename of ["AGENTS.md", "SKILL.md", "README.md", "CLAUDE.md"]) {
        if (source.includes(filename)) expect(text).toContain(filename);
      }
    }
  });
});
