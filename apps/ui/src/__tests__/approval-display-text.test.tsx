import { render, screen } from "@testing-library/react";
import { ApprovalDisplayText } from "@/components/chat/approval-display-text";

// Enumerate all Unicode Bidi_Control characters, including directional marks.
const bidiControls = [
  0x061c, 0x200e, 0x200f, 0x202a, 0x202b, 0x202c, 0x202d, 0x202e, 0x2066, 0x2067, 0x2068, 0x2069,
];
const otherControls = [
  ...Array.from({ length: 32 }, (_, i) => i).filter((i) => i !== 9 && i !== 10),
  ...Array.from({ length: 33 }, (_, i) => 0x7f + i),
  0x2028,
  0x2029,
];

it.each([...bidiControls, ...otherControls])(
  "reveals control U+%s as an accessible escape",
  (code) => {
    const char = String.fromCharCode(code);
    const hex = code.toString(16).toUpperCase().padStart(4, "0");
    const { container } = render(<ApprovalDisplayText>{`before${char}after`}</ApprovalDisplayText>);
    expect(container.textContent).toBe(`before\\u${hex}after`);
    const marker = screen.getByRole("img", { name: `Unicode control character U+${hex}` });
    expect(marker).toHaveAttribute("dir", "ltr");
    expect(marker).toHaveAttribute("title", `Unicode control character U+${hex}`);
  },
);

it("preserves ordinary scripts, Persian joiners, emoji, indentation and literal escapes", () => {
  const text = 'می‌خواهم فارسی العربية עברית 日本語 👩‍💻 ❤️ 🏳️‍🌈\n\t  echo "\\u202E"';
  const { container } = render(<ApprovalDisplayText>{text}</ApprovalDisplayText>);
  expect(container.textContent).toBe(text);
  expect(screen.queryByRole("img")).not.toBeInTheDocument();
});

it("distinguishes actual controls from literal escape text and escapes HTML as text", () => {
  const { container } = render(
    <ApprovalDisplayText>{"\\u202E\u202E\u2069<img src=x onerror=alert(1)>"}</ApprovalDisplayText>,
  );
  expect(container.textContent).toBe("\\u202E\\u202E\\u2069<img src=x onerror=alert(1)>");
  expect(screen.getAllByRole("img")).toHaveLength(2);
  expect(container.querySelector("img")).toBeNull();
});
