import { hasA2UIBlocks, hasOnlyA2UIBlocks, parseA2UI, splitA2UIBlocks } from "@/lib/a2ui-utils";

describe("hasA2UIBlocks", () => {
  it("returns true for a2ui fence", () => {
    expect(hasA2UIBlocks('```a2ui\n{"type":"Card"}\n```')).toBe(true);
  });

  it("returns true for openui fence", () => {
    expect(hasA2UIBlocks("```openui\nroot = Card([])\n```")).toBe(true);
  });

  it("returns false for unrelated fences", () => {
    expect(hasA2UIBlocks("```python\nprint(1)\n```")).toBe(false);
  });

  it("returns false for empty text", () => {
    expect(hasA2UIBlocks("")).toBe(false);
  });
});

describe("hasOnlyA2UIBlocks", () => {
  it("distinguishes a2ui from openui", () => {
    expect(hasOnlyA2UIBlocks("```a2ui\n{}\n```")).toBe(true);
    expect(hasOnlyA2UIBlocks("```openui\nroot = Card()\n```")).toBe(false);
  });
});

describe("splitA2UIBlocks", () => {
  it("returns whole text as markdown when no blocks present", () => {
    expect(splitA2UIBlocks("Hello")).toEqual([{ type: "markdown", content: "Hello" }]);
  });

  it("splits an a2ui block surrounded by markdown", () => {
    const text = 'Before\n```a2ui\n{"type":"Card"}\n```\nAfter';
    expect(splitA2UIBlocks(text)).toEqual([
      { type: "markdown", content: "Before\n" },
      { type: "a2ui", content: '{"type":"Card"}\n' },
      { type: "markdown", content: "\nAfter" },
    ]);
  });

  it("interleaves a2ui and openui blocks correctly", () => {
    const text = '```a2ui\n{"type":"Card"}\n```\nmiddle\n```openui\nroot = Card([])\n```';
    const segs = splitA2UIBlocks(text);
    expect(segs.map((s) => s.type)).toEqual(["a2ui", "markdown", "openui"]);
  });

  it("handles streaming a2ui (no closing fence)", () => {
    const text = 'Here:\n```a2ui\n{"type":"Card"';
    expect(splitA2UIBlocks(text)).toEqual([
      { type: "markdown", content: "Here:\n" },
      { type: "a2ui", content: '{"type":"Card"' },
    ]);
  });

  it("skips empty blocks", () => {
    const text = "Before\n```a2ui\n\n```\nAfter";
    expect(splitA2UIBlocks(text)).toEqual([
      { type: "markdown", content: "Before\n" },
      { type: "markdown", content: "\nAfter" },
    ]);
  });
});

describe("parseA2UI", () => {
  it("parses valid JSON", () => {
    const result = parseA2UI('{"type":"Card","props":{"variant":"muted"}}');
    expect(result).toEqual({ type: "Card", props: { variant: "muted" } });
  });

  it("returns null for empty input", () => {
    expect(parseA2UI("")).toBeNull();
    expect(parseA2UI("   ")).toBeNull();
  });

  it("returns null for completely garbage input", () => {
    expect(parseA2UI("not json at all")).toBeNull();
  });

  it("recovers a parseable prefix from truncated JSON", () => {
    // Streaming: object opened but not closed.
    const partial = '{"type":"Card","children":[{"type":"Heading","props":{"text":"Hi"}';
    const result = parseA2UI(partial);
    expect(result).toBeTruthy();
    // Should at least have recovered the outer type.
    expect((result as { type: string }).type).toBe("Card");
  });

  it("handles unterminated strings by truncating", () => {
    const partial = '{"type":"Card","props":{"variant":"muted';
    const result = parseA2UI(partial);
    // May recover outer type, may be null — either is acceptable as long as
    // it doesn't throw.
    if (result !== null) {
      expect((result as { type: string }).type).toBe("Card");
    }
  });

  it("parses nested children", () => {
    const json = JSON.stringify({
      type: "Card",
      children: [
        { type: "Heading", props: { text: "Title" } },
        { type: "Text", props: { text: "Body" } },
      ],
    });
    const result = parseA2UI(json) as { children: unknown[] };
    expect(result.children).toHaveLength(2);
  });
});
