import { fireEvent, render, screen } from "@testing-library/react";
import * as mockReact from "react";
import { MemoryConfigEditor, readAttachments } from "@/components/agents/memory-config-editor";
import { memoryStore } from "./fixtures/memory";

jest.mock("@/hooks/use-memory", () => ({
  useMemoryStores: () => ({
    data: [memoryStore(), memoryStore({ id: "mem_x", name: "org", store: "org" })],
    isLoading: false,
  }),
}));

jest.mock("@/components/ui/select", () => {
  function collect(node: mockReact.ReactNode): string[] {
    const values: string[] = [];
    mockReact.Children.forEach(node, (child: mockReact.ReactNode) => {
      if (!mockReact.isValidElement(child)) return;
      const props = child.props as { value?: string; children?: mockReact.ReactNode };
      if ((child.type as { isItem?: boolean }).isItem && props.value) values.push(props.value);
      else values.push(...collect(props.children));
    });
    return values;
  }
  function Select({
    value,
    onValueChange,
    children,
  }: {
    value: string;
    onValueChange: (value: string) => void;
    children: mockReact.ReactNode;
  }) {
    const label = mockReact.Children.toArray(children)
      .map((child) =>
        mockReact.isValidElement(child)
          ? (child.props as { "aria-label"?: string })["aria-label"]
          : undefined,
      )
      .find(Boolean);
    return (
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onValueChange(event.target.value)}
      >
        {collect(children).map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
    );
  }
  function SelectItem() {
    return null;
  }
  SelectItem.isItem = true;
  return {
    Select,
    SelectContent: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectItem,
    SelectTrigger: (props: { "aria-label"?: string }) => <span {...props} />,
    SelectValue: () => null,
  };
});

describe("MemoryConfigEditor", () => {
  it("shows server defaults and writes one switch at a time", () => {
    const onChange = jest.fn();
    render(<MemoryConfigEditor config={{}} onChange={onChange} />);
    expect((screen.getByLabelText("Organization access") as HTMLSelectElement).value).toBe("read");
    expect((screen.getByLabelText("Team access") as HTMLSelectElement).value).toBe("read_write");
    fireEvent.change(screen.getByLabelText("Organization access"), { target: { value: "off" } });
    expect(onChange).toHaveBeenCalledWith({ org: "off" });
  });

  it("offers parent memory read-only", () => {
    render(<MemoryConfigEditor config={{}} onChange={jest.fn()} />);
    const options = Array.from(
      (screen.getByLabelText("Parent access") as HTMLSelectElement).options,
    ).map((option) => option.value);
    expect(options).toEqual(["off", "read"]);
  });

  it("attaches named memories and turns off the card", () => {
    const onChange = jest.fn();
    render(<MemoryConfigEditor config={{ team: "off" }} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /Attach a named memory/ }));
    expect(onChange).toHaveBeenLastCalledWith({
      team: "off",
      attach: [{ memory: "", mode: "read" }],
    });
    fireEvent.click(screen.getByRole("switch"));
    expect(onChange).toHaveBeenLastCalledWith({ team: "off", card: false });
  });

  it("reads only well-formed attachments", () => {
    expect(
      readAttachments({
        attach: [{ memory: "everruns-maintenance", mode: "read_write" }, { mode: "read" }, "x"],
      }),
    ).toEqual([{ memory: "everruns-maintenance", mode: "read_write" }]);
  });
});
