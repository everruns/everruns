import {
  injectCitationMarkers,
  numberCitations,
  parseCitationHref,
} from "@/components/chat/message-citations";
import type { TextAnnotation } from "@/lib/api/schema-types";

function annotation(
  start: number,
  end: number,
  uri: string,
  extra: Partial<TextAnnotation> = {},
): TextAnnotation {
  return {
    start,
    end,
    origin: "citation_retrieval",
    source: { uri, title: null, snippet: null, location: null },
    external_id: null,
    verified: null,
    ...extra,
  };
}

describe("numberCitations", () => {
  it("numbers unique sources by first appearance and dedupes by uri", () => {
    const anns = [annotation(0, 5, "u1"), annotation(6, 9, "u2"), annotation(10, 14, "u1")];
    const { sources, numberByAnnotation } = numberCitations(anns);
    expect(sources.map((s) => s.n)).toEqual([1, 2]);
    expect(sources.map((s) => s.uri)).toEqual(["u1", "u2"]);
    // Third annotation reuses u1's number.
    expect(numberByAnnotation).toEqual([1, 2, 1]);
  });

  it("fills missing snippet/verdict from a later annotation of the same source", () => {
    const anns = [
      annotation(0, 5, "u1"),
      annotation(6, 9, "u1", {
        source: { uri: "u1", title: null, snippet: "hello", location: null },
        verified: { status: "entailed", score: 0.9 },
      }),
    ];
    const { sources } = numberCitations(anns);
    expect(sources).toHaveLength(1);
    expect(sources[0].snippet).toBe("hello");
    expect(sources[0].verified?.status).toBe("entailed");
  });
});

describe("injectCitationMarkers", () => {
  it("inserts markers at end offsets without shifting earlier ones", () => {
    const text = "Alpha beta gamma";
    // "Alpha" ends at 5, "gamma" ends at 16.
    const anns = [annotation(0, 5, "u1"), annotation(11, 16, "u2")];
    const out = injectCitationMarkers(text, anns);
    expect(out).toBe("Alpha[1](#cite-1) beta gamma[2](#cite-2)");
  });

  it("drops out-of-range offsets", () => {
    const text = "short";
    const anns = [annotation(0, 99, "u1")];
    expect(injectCitationMarkers(text, anns)).toBe("short");
  });
});

describe("parseCitationHref", () => {
  it("extracts the citation number", () => {
    expect(parseCitationHref("#cite-3")).toBe(3);
  });
  it("returns null for non-citation hrefs", () => {
    expect(parseCitationHref("https://example.com")).toBeNull();
    expect(parseCitationHref(undefined)).toBeNull();
  });
});
