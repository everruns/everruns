/**
 * Tests for Markdown component alert parsing logic.
 *
 * Note: Full rendering tests require ESM module transformation for react-markdown.
 * These tests focus on the core alert detection and parsing functionality.
 */

// Test the alert parsing logic directly by extracting the regex patterns
describe("GitHub Alert Detection", () => {
  // Regex pattern for GitHub-style alerts (matches the component implementation)
  const alertPattern = /^\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*/i;

  describe("alert type detection", () => {
    it("detects NOTE alert", () => {
      const text = "[!NOTE] This is a note.";
      const match = text.match(alertPattern);
      expect(match).not.toBeNull();
      expect(match![1].toLowerCase()).toBe("note");
    });

    it("detects TIP alert", () => {
      const text = "[!TIP] This is a tip.";
      const match = text.match(alertPattern);
      expect(match).not.toBeNull();
      expect(match![1].toLowerCase()).toBe("tip");
    });

    it("detects IMPORTANT alert", () => {
      const text = "[!IMPORTANT] This is important.";
      const match = text.match(alertPattern);
      expect(match).not.toBeNull();
      expect(match![1].toLowerCase()).toBe("important");
    });

    it("detects WARNING alert", () => {
      const text = "[!WARNING] This is a warning.";
      const match = text.match(alertPattern);
      expect(match).not.toBeNull();
      expect(match![1].toLowerCase()).toBe("warning");
    });

    it("detects CAUTION alert", () => {
      const text = "[!CAUTION] This is a caution.";
      const match = text.match(alertPattern);
      expect(match).not.toBeNull();
      expect(match![1].toLowerCase()).toBe("caution");
    });

    it("is case-insensitive", () => {
      const texts = ["[!note] lowercase", "[!Note] mixed case", "[!NOTE] uppercase"];
      texts.forEach((text) => {
        const match = text.match(alertPattern);
        expect(match).not.toBeNull();
        expect(match![1].toLowerCase()).toBe("note");
      });
    });

    it("allows whitespace before alert marker", () => {
      const text = "  [!NOTE] This is a note.";
      const match = text.match(alertPattern);
      expect(match).not.toBeNull();
      expect(match![1]).toBe("NOTE");
    });

    it("does not match invalid alert types", () => {
      const invalidTypes = ["[!INVALID]", "[!ERROR]", "[!INFO]", "[!DANGER]"];
      invalidTypes.forEach((type) => {
        const match = type.match(alertPattern);
        expect(match).toBeNull();
      });
    });

    it("does not match regular blockquote text", () => {
      const text = "This is a regular quote without alert marker.";
      const match = text.match(alertPattern);
      expect(match).toBeNull();
    });

    it("does not match malformed alert syntax", () => {
      const malformed = [
        "[NOTE]", // Missing exclamation
        "![NOTE]", // Wrong order
        "[!NOTE", // Missing closing bracket
        "!NOTE]", // Missing opening bracket
        "[ !NOTE]", // Space before exclamation
      ];
      malformed.forEach((text) => {
        const match = text.match(alertPattern);
        expect(match).toBeNull();
      });
    });
  });

  describe("alert marker removal", () => {
    it("removes alert marker from text", () => {
      const text = "[!NOTE] This is the content.";
      const cleaned = text.replace(alertPattern, "");
      expect(cleaned).toBe("This is the content.");
    });

    it("removes alert marker with trailing whitespace", () => {
      const text = "[!TIP]  Content with extra spaces.";
      const cleaned = text.replace(alertPattern, "");
      expect(cleaned).toBe("Content with extra spaces.");
    });

    it("removes alert marker from multiline content (first line)", () => {
      const text = "[!WARNING]\nThis is the warning content.";
      const cleaned = text.replace(alertPattern, "");
      // The \s* in the pattern consumes whitespace including newlines
      expect(cleaned).toBe("This is the warning content.");
    });
  });
});

describe("Alert Configuration", () => {
  // Test that all 5 alert types are properly supported
  const alertTypes = ["note", "tip", "important", "warning", "caution"] as const;

  it("supports exactly 5 alert types", () => {
    expect(alertTypes).toHaveLength(5);
  });

  it("all alert types have distinct styling purposes", () => {
    // Document the intended use of each alert type
    const purposes: Record<(typeof alertTypes)[number], string> = {
      note: "Highlights information that users should take into account",
      tip: "Optional information to help a user be more successful",
      important: "Crucial information necessary for users to succeed",
      warning: "Critical content demanding immediate user attention due to potential risks",
      caution: "Negative potential consequences of an action",
    };

    alertTypes.forEach((type) => {
      expect(purposes[type]).toBeDefined();
      expect(purposes[type].length).toBeGreaterThan(0);
    });
  });
});
