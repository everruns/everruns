import { buildImageSrc } from "@/components/chat/read-file-tool-call-card";

describe("buildImageSrc", () => {
  describe("URL inputs", () => {
    it("allows https URLs without a declared media_type", () => {
      expect(buildImageSrc({ url: "https://example.com/photo.png" })).toBe(
        "https://example.com/photo.png",
      );
    });

    it("allows http URLs with an allowed raster media_type", () => {
      expect(
        buildImageSrc({ url: "http://cdn.example.com/img.webp", media_type: "image/webp" }),
      ).toBe("http://cdn.example.com/img.webp");
    });

    it("rejects SVG remote URLs when media_type declares image/svg+xml", () => {
      expect(
        buildImageSrc({ url: "https://example.com/evil.svg", media_type: "image/svg+xml" }),
      ).toBeNull();
    });

    it("rejects data: URLs", () => {
      expect(
        buildImageSrc({ url: "data:image/png;base64,abc123" }),
      ).toBeNull();
    });

    it("rejects javascript: URLs", () => {
      expect(buildImageSrc({ url: "javascript:alert(1)" })).toBeNull();
    });

    it("rejects blob: URLs", () => {
      expect(buildImageSrc({ url: "blob:https://example.com/uuid" })).toBeNull();
    });
  });

  describe("base64 inputs", () => {
    it("allows known raster types", () => {
      for (const mime of ["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"]) {
        expect(buildImageSrc({ base64: "abc123", media_type: mime })).toBe(
          `data:${mime};base64,abc123`,
        );
      }
    });

    it("rejects SVG data URLs", () => {
      expect(
        buildImageSrc({ base64: "PHN2Zy8+", media_type: "image/svg+xml" }),
      ).toBeNull();
    });

    it("rejects unknown/arbitrary media types", () => {
      expect(buildImageSrc({ base64: "abc", media_type: "application/javascript" })).toBeNull();
      expect(buildImageSrc({ base64: "abc", media_type: "text/html" })).toBeNull();
    });

    it("defaults to image/png when no media_type is provided", () => {
      expect(buildImageSrc({ base64: "abc123" })).toBe("data:image/png;base64,abc123");
    });

    it("is case-insensitive for media_type", () => {
      expect(buildImageSrc({ base64: "abc", media_type: "Image/SVG+XML" })).toBeNull();
      expect(buildImageSrc({ base64: "abc", media_type: "IMAGE/PNG" })).toBe(
        "data:IMAGE/PNG;base64,abc",
      );
    });

    it("strips media_type parameters before comparing", () => {
      expect(
        buildImageSrc({ base64: "abc", media_type: "image/svg+xml; charset=utf-8" }),
      ).toBeNull();
      expect(buildImageSrc({ base64: "abc", media_type: "image/png; charset=utf-8" })).toBe(
        "data:image/png; charset=utf-8;base64,abc",
      );
    });
  });

  describe("empty / missing inputs", () => {
    it("returns null when both url and base64 are absent", () => {
      expect(buildImageSrc({})).toBeNull();
    });

    it("returns null when url is empty string", () => {
      expect(buildImageSrc({ url: "" })).toBeNull();
    });

    it("returns null when base64 is empty string", () => {
      expect(buildImageSrc({ base64: "" })).toBeNull();
    });
  });
});
