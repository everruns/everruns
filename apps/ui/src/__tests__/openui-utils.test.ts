import { splitOpenUIBlocks, hasOpenUIBlocks } from "@/lib/openui-utils";

describe("hasOpenUIBlocks", () => {
  it("returns true when text contains an openui code block", () => {
    const text = "Hello\n```openui\nroot = Card([])\n```\nBye";
    expect(hasOpenUIBlocks(text)).toBe(true);
  });

  it("returns false for plain text", () => {
    expect(hasOpenUIBlocks("Just some text")).toBe(false);
  });

  it("returns false for other code blocks", () => {
    expect(hasOpenUIBlocks("```python\nprint('hi')\n```")).toBe(false);
  });

  it("returns false for empty string", () => {
    expect(hasOpenUIBlocks("")).toBe(false);
  });

  it("returns true even without closing backticks (streaming)", () => {
    expect(hasOpenUIBlocks("```openui\nroot = Card([])\n")).toBe(true);
  });
});

describe("splitOpenUIBlocks", () => {
  it("returns whole text as markdown when no openui blocks", () => {
    const segments = splitOpenUIBlocks("Hello world");
    expect(segments).toEqual([{ type: "markdown", content: "Hello world" }]);
  });

  it("splits text with a single openui block", () => {
    const text = "Before\n```openui\nroot = Card([])\n```\nAfter";
    const segments = splitOpenUIBlocks(text);
    expect(segments).toEqual([
      { type: "markdown", content: "Before\n" },
      { type: "openui", content: "root = Card([])\n" },
      { type: "markdown", content: "\nAfter" },
    ]);
  });

  it("handles openui block at the start", () => {
    const text = "```openui\nroot = Card([])\n```\nAfter";
    const segments = splitOpenUIBlocks(text);
    expect(segments).toEqual([
      { type: "openui", content: "root = Card([])\n" },
      { type: "markdown", content: "\nAfter" },
    ]);
  });

  it("handles openui block at the end", () => {
    const text = "Before\n```openui\nroot = Card([])\n```";
    const segments = splitOpenUIBlocks(text);
    expect(segments).toEqual([
      { type: "markdown", content: "Before\n" },
      { type: "openui", content: "root = Card([])\n" },
    ]);
  });

  it("handles multiple openui blocks", () => {
    const text =
      "Intro\n```openui\nroot = Card([])\n```\nMiddle\n```openui\nroot = Table([])\n```\nEnd";
    const segments = splitOpenUIBlocks(text);
    expect(segments).toEqual([
      { type: "markdown", content: "Intro\n" },
      { type: "openui", content: "root = Card([])\n" },
      { type: "markdown", content: "\nMiddle\n" },
      { type: "openui", content: "root = Table([])\n" },
      { type: "markdown", content: "\nEnd" },
    ]);
  });

  it("handles streaming (no closing backticks)", () => {
    const text = "Here:\n```openui\nroot = Card([])";
    const segments = splitOpenUIBlocks(text);
    expect(segments).toEqual([
      { type: "markdown", content: "Here:\n" },
      { type: "openui", content: "root = Card([])" },
    ]);
  });

  it("returns empty array for empty string", () => {
    const segments = splitOpenUIBlocks("");
    expect(segments).toEqual([]);
  });

  it("returns empty array for whitespace-only string", () => {
    const segments = splitOpenUIBlocks("   \n  ");
    expect(segments).toEqual([]);
  });

  it("skips empty openui blocks", () => {
    const text = "Before\n```openui\n   \n```\nAfter";
    const segments = splitOpenUIBlocks(text);
    expect(segments).toEqual([
      { type: "markdown", content: "Before\n" },
      { type: "markdown", content: "\nAfter" },
    ]);
  });

  it("handles multiline openui content", () => {
    const code = 'items = [\n  Text("Hello"),\n  Text("World"),\n]\nroot = Card(items)\n';
    const text = `Before\n\`\`\`openui\n${code}\`\`\`\nAfter`;
    const segments = splitOpenUIBlocks(text);
    expect(segments).toHaveLength(3);
    expect(segments[1]).toEqual({ type: "openui", content: code });
  });

  it("only returns openui block when there is no surrounding text", () => {
    const text = "```openui\nroot = Card([])\n```";
    const segments = splitOpenUIBlocks(text);
    expect(segments).toEqual([{ type: "openui", content: "root = Card([])\n" }]);
  });
});
