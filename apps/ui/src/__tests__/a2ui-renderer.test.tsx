import { fireEvent, render, screen } from "@testing-library/react";
import { LegacyA2UI } from "../../generative-ui/src/legacy";

jest.mock("streamdown", () => ({
  Streamdown: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));

const send = jest.fn();
const renderTree = (tree: unknown, onMessage: ((text: string) => void) | undefined = send) =>
  render(<LegacyA2UI code={JSON.stringify(tree)} onMessage={onMessage} />);
beforeEach(() => send.mockClear());

test("legacy trees keep heading levels, badges and message actions", () => {
  renderTree({
    type: "Stack",
    children: [
      { type: "Heading", props: { text: "Report", level: 2 } },
      { type: "Badge", props: { label: "Ready" } },
      {
        type: "Button",
        props: { label: "Explain", action: { type: "message", text: "Explain this report" } },
      },
    ],
  });
  expect(screen.getByRole("heading", { level: 2, name: "Report" })).toBeVisible();
  expect(screen.getByText("Ready")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Explain" }));
  expect(send).toHaveBeenCalledWith("Explain this report");
});
test("form submission includes unchanged defaults, edits and unchecked values", () => {
  renderTree({
    type: "Form",
    props: { name: "Review", submitLabel: "Send review" },
    children: [
      { type: "TextField", props: { name: "scope", label: "Scope", defaultValue: "All" } },
      { type: "Textarea", props: { name: "notes", label: "Notes" } },
      { type: "Checkbox", props: { name: "approved", label: "Approved", defaultChecked: true } },
    ],
  });
  fireEvent.change(screen.getByLabelText("Notes"), { target: { value: "Ready" } });
  fireEvent.click(screen.getByLabelText("Approved"));
  fireEvent.click(screen.getByRole("button", { name: "Send review" }));
  expect(send).toHaveBeenCalledWith(
    'Submitted Review: {"scope":"All","notes":"Ready","approved":false}',
  );
});
test.each(["javascript:alert(1)", "data:text/html,<script>alert(1)</script>"])(
  "rejects unsafe image and link URL %s",
  (url) => {
    renderTree({
      type: "Stack",
      children: [
        { type: "Image", props: { src: url, alt: "Image" } },
        { type: "Button", props: { label: "Open", action: { type: "open_url", url } } },
      ],
    });
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(screen.getByRole("button")).toBeDisabled();
  },
);
test("safe URL actions are ordinary links with opener isolation", () => {
  renderTree({
    type: "Button",
    props: { label: "Open", action: { type: "open_url", url: "https://example.com" } },
  });
  expect(screen.getByRole("link", { name: "Open" })).toHaveAttribute("rel", "noopener noreferrer");
});
test("read-only forms cannot send", () => {
  render(<LegacyA2UI code={JSON.stringify({ type: "Form", children: [] })} />);
  expect(screen.getByRole("button", { name: "Submit" })).toBeDisabled();
});
test("partial legacy JSON can render before the children finish", () => {
  render(<LegacyA2UI code={'{"type":"Card","children":['} />);
  expect(screen.queryByText(/"type"/)).not.toBeInTheDocument();
});
test("unrecoverable JSON falls back to source", () => {
  render(<LegacyA2UI code="not json" />);
  expect(screen.getByText("not json")).toBeVisible();
});
