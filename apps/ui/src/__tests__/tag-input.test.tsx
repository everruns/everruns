import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";

import { TagInput } from "@/components/ui/tag-input";

function ControlledTagInput({ initialValue = "" }: { initialValue?: string }) {
  const [value, setValue] = useState(initialValue);
  return (
    <>
      <label htmlFor="tags">Tags</label>
      <TagInput id="tags" value={value} onChange={setValue} />
      <output data-testid="value">{value}</output>
    </>
  );
}

describe("TagInput", () => {
  it("renders existing tags as removable chips", () => {
    render(<ControlledTagInput initialValue="ops, alerts" />);

    expect(screen.getByText("ops")).toBeInTheDocument();
    expect(screen.getByText("alerts")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Remove ops" }));

    expect(screen.getByTestId("value")).toHaveTextContent("alerts");
    expect(screen.queryByText("ops")).not.toBeInTheDocument();
  });

  it("adds trimmed tags with Enter and ignores duplicates", () => {
    render(<ControlledTagInput initialValue="ops" />);
    const input = screen.getByLabelText("Tags");

    fireEvent.change(input, { target: { value: " alerts " } });
    fireEvent.keyDown(input, { key: "Enter" });
    fireEvent.change(input, { target: { value: "ops" } });
    fireEvent.keyDown(input, { key: "," });

    expect(screen.getByTestId("value")).toHaveTextContent("ops, alerts");
    expect(screen.getAllByText("ops")).toHaveLength(1);
    expect(input).toHaveValue("");
  });

  it("splits pasted comma-separated tags", () => {
    render(<ControlledTagInput />);
    const input = screen.getByLabelText("Tags");

    fireEvent.paste(input, {
      clipboardData: { getData: () => "demo, support, demo" },
    });

    expect(screen.getByTestId("value")).toHaveTextContent("demo, support");
  });

  it("removes the last tag with Backspace from an empty input", () => {
    render(<ControlledTagInput initialValue="ops, alerts" />);

    fireEvent.keyDown(screen.getByLabelText("Tags"), { key: "Backspace" });

    expect(screen.getByTestId("value")).toHaveTextContent("ops");
    expect(screen.queryByText("alerts")).not.toBeInTheDocument();
  });
});
