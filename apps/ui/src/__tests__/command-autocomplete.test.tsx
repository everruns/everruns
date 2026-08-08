import { fireEvent, render, screen } from "@testing-library/react";
import { CommandAutocomplete } from "@/components/chat/command-autocomplete";
import type { CommandDescriptor } from "@/lib/api/types";

const commands: CommandDescriptor[] = [
  {
    name: "btw",
    description: "Ask a side question",
    source: "system",
    args: [{ name: "question", description: "Question", required: true }],
  },
  {
    name: "review",
    description: "Review the current work",
    source: "skill",
  },
];

beforeAll(() => {
  Object.defineProperty(window.HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: jest.fn(),
  });
});

describe("CommandAutocomplete", () => {
  it("exposes an accessible, filterable command list", () => {
    render(
      <CommandAutocomplete
        commands={commands}
        inputValue="/b"
        visible
        onSelect={jest.fn()}
        onDismiss={jest.fn()}
      />,
    );

    expect(screen.getByRole("listbox", { name: "Available commands" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /\/btw/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("option", { name: /\/review/ })).not.toBeInTheDocument();
  });

  it("supports keyboard selection and dismissal", () => {
    const onSelect = jest.fn();
    const onDismiss = jest.fn();
    render(
      <CommandAutocomplete
        commands={commands}
        inputValue="/"
        visible
        onSelect={onSelect}
        onDismiss={onDismiss}
      />,
    );

    fireEvent.keyDown(document, { key: "ArrowDown" });
    fireEvent.keyDown(document, { key: "Enter" });
    expect(onSelect).toHaveBeenCalledWith(commands[1]);

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onDismiss).toHaveBeenCalled();
  });

  it("supports pointer selection and explains an empty filter", () => {
    const onSelect = jest.fn();
    const { rerender } = render(
      <CommandAutocomplete
        commands={commands}
        inputValue="/"
        visible
        onSelect={onSelect}
        onDismiss={jest.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("option", { name: /\/btw/ }));
    expect(onSelect).toHaveBeenCalledWith(commands[0]);

    rerender(
      <CommandAutocomplete
        commands={commands}
        inputValue="/missing"
        visible
        onSelect={onSelect}
        onDismiss={jest.fn()}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("No matching commands");
  });
});
