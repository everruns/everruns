import { toLines } from "@/components/session/trace/json-view";

describe("JsonView lines", () => {
  it("shows a multi-line string as rows of its text", () => {
    const source = JSON.stringify({ content: "1|one\n2|two", ok: true }, null, 2);
    expect(toLines(source)).toEqual([
      { text: "{" },
      { text: '  "content":' },
      { text: "    1|one", block: true },
      { text: "    2|two", block: true },
      { text: '  "ok": true' },
      { text: "}" },
    ]);
  });

  it("leaves single-line strings and other values as JSON", () => {
    const source = JSON.stringify({ path: "a.txt", n: 1 }, null, 2);
    expect(toLines(source).map((l) => l.text)).toEqual(source.split("\n"));
    expect(toLines(source).some((l) => l.block)).toBe(false);
  });
});
