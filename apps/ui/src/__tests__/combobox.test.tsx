import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Combobox } from "@/components/ui/combobox";

const options = [
  { value: "user-a", label: "Alice" },
  { value: "user-b", label: "Bob", disabled: true },
  { value: "user-c", label: "Carol" },
];
function Picker({ disabled = false }: { disabled?: boolean }) {
  const [value, setValue] = useState("user-a");
  return (
    <Combobox
      options={options}
      value={value}
      onValueChange={setValue}
      ariaLabel="Person"
      searchPlaceholder="Search people"
      disabled={disabled}
    />
  );
}

it("searches labels and identifiers, skips disabled results, and resets on reopening", async () => {
  render(<Picker />);
  fireEvent.click(screen.getByRole("combobox", { name: "Person" }));
  const input = await screen.findByRole("combobox", { name: "Search people" });
  fireEvent.change(input, { target: { value: "user-c" } });
  expect(screen.queryByRole("option", { name: "Alice" })).not.toBeInTheDocument();
  expect(screen.getByRole("option", { name: "Carol" })).toBeInTheDocument();
  fireEvent.keyDown(input, { key: "ArrowDown" });
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(screen.queryByRole("listbox")).not.toBeInTheDocument());
  expect(screen.getByRole("combobox", { name: "Person" })).toHaveTextContent("Carol");
  fireEvent.click(screen.getByRole("combobox", { name: "Person" }));
  expect(await screen.findByRole("combobox", { name: "Search people" })).toHaveValue("");
  expect(screen.getByRole("option", { name: "Bob" })).toHaveAttribute("aria-disabled", "true");
  fireEvent.change(screen.getByRole("combobox", { name: "Search people" }), {
    target: { value: "nobody" },
  });
  expect(screen.getByRole("status")).toHaveTextContent("No results found");
  fireEvent.keyDown(screen.getByRole("combobox", { name: "Search people" }), { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("listbox")).not.toBeInTheDocument());
  expect(screen.getByRole("combobox", { name: "Person" })).toHaveTextContent("Carol");
});

it("cannot open when disabled", () => {
  render(<Picker disabled />);
  expect(screen.getByRole("combobox", { name: "Person" })).toBeDisabled();
  fireEvent.click(screen.getByRole("combobox", { name: "Person" }));
  expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
});

it("keeps the selected label while server results load or fail and leaves paging reachable", async () => {
  const search = jest.fn();
  const props = {
    options: [],
    value: "user-a",
    valueLabel: "Alice",
    onValueChange: jest.fn(),
    onSearchChange: search,
    ariaLabel: "Person",
    footer: <button>Load more people</button>,
  };
  const view = render(<Combobox {...props} loading />);
  fireEvent.click(screen.getByRole("combobox", { name: "Person" }));
  const input = await screen.findByRole("combobox", { name: "Search..." });
  fireEvent.change(input, { target: { value: "Carol" } });
  expect(search).toHaveBeenLastCalledWith("Carol");
  expect(screen.getByRole("status")).toHaveTextContent("Loading");
  expect(screen.getByRole("combobox", { name: "Person" })).toHaveTextContent("Alice");
  view.rerender(<Combobox {...props} error="Could not load people" />);
  expect(screen.getByRole("alert")).toHaveTextContent("Could not load people");
  expect(screen.queryByText("No results found")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Load more people" })).toBeEnabled();
  expect(props.onValueChange).not.toHaveBeenCalled();
});

it("connects a visible field label and validation hint to the trigger", () => {
  render(
    <>
      <label htmlFor="resource">Bound resource</label>
      <p id="resource-error">Choose an available resource</p>
      <Combobox
        id="resource"
        aria-invalid
        aria-describedby="resource-error"
        options={options}
        value="user-a"
        onValueChange={jest.fn()}
      />
    </>,
  );
  const trigger = screen.getByRole("combobox", { name: "Bound resource" });
  expect(trigger).toHaveAttribute("aria-invalid", "true");
  expect(trigger).toHaveAccessibleDescription("Choose an available resource");
});
