import { uiMessages } from "@/lib/ui-messages";

const tokens = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();

describe("Ukrainian product catalog", () => {
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
