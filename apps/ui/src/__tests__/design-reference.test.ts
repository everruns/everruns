import reference from "../../design-reference.json";

describe("design reference provenance", () => {
  it("pins the adopted archive and its review surfaces", () => {
    expect(reference.archiveFile).toBe("Everruns Design System.zip");
    expect(reference.archiveSha256).toMatch(/^[a-f0-9]{64}$/);
    expect(reference.sourceUrl).toMatch(/^https:\/\/claude\.ai\/design\/p\//);
    expect(reference.canonicalRuntime).toBe("src/app/design-system.css");
    expect(reference.canonicalGuidance).toBe("DESIGN.md");
    expect(reference.referenceShowcase).toBe("/dev/design-reference");
    expect(reference.componentShowcase).toBe("/dev/ui-components");
  });
});
