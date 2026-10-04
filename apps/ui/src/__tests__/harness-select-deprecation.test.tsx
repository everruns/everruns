import { fireEvent, render, screen } from "@testing-library/react";
import { HarnessSelect } from "@/components/harness/harness-select";

jest.mock("@/hooks", () => ({
  useHarnesses: () => ({
    data: [
      {
        id: "conversation",
        name: "conversation",
        display_name: "Conversation",
        is_built_in: true,
        tags: [],
        status: "active",
      },
      {
        id: "legacy",
        name: "generic",
        display_name: "Generic — deprecated",
        is_built_in: true,
        tags: ["deprecated"],
        status: "active",
      },
    ],
  }),
}));

describe("deprecated harness selection", () => {
  it("keeps an existing deprecated selection readable", () => {
    render(<HarnessSelect value="legacy" onValueChange={jest.fn()} />);
    expect(screen.getByRole("combobox")).toHaveTextContent("Generic — deprecated");
  });

  it("hides legacy choices until requested without changing the selected harness", () => {
    const onChange = jest.fn();
    render(<HarnessSelect value="conversation" onValueChange={onChange} />);
    fireEvent.click(screen.getByRole("combobox"));
    expect(screen.queryByRole("option", { name: "Generic — deprecated" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show deprecated" }));
    expect(screen.getByRole("option", { name: "Generic — deprecated" })).toBeInTheDocument();
    expect(onChange).not.toHaveBeenCalled();
  });
});
