import { fireEvent, render, screen } from "@testing-library/react";
import { HarnessSelect } from "@/components/harness/harness-select";
import { useHarnesses } from "@/hooks";
import type { Harness } from "@/lib/api/types";

jest.mock("@/hooks", () => ({
  useHarnesses: jest.fn(),
}));

const mockUseHarnesses = jest.mocked(useHarnesses);

function harness(overrides: Partial<Harness> & Pick<Harness, "id" | "name">): Harness {
  return {
    display_name: null,
    description: null,
    parent_harness_id: null,
    default_model_id: null,
    tags: [],
    capabilities: [],
    is_built_in: true,
    status: "active",
    created_at: "2026-08-07T00:00:00Z",
    updated_at: "2026-08-07T00:00:00Z",
    archived_at: null,
    deleted_at: null,
    ...overrides,
  };
}

const generic = harness({
  id: "harness_generic",
  name: "generic",
  display_name: "Generic",
  description: "Everyday agents: files, a shell, the web, and the usual safeguards.",
});
const base = harness({
  id: "harness_base",
  name: "base",
  display_name: "Base",
  description: "A blank start. Add only the capabilities this agent should have.",
});
const untitled = harness({
  id: "harness_custom",
  name: "custom",
  display_name: "Custom",
});

describe("HarnessSelect", () => {
  beforeEach(() => {
    mockUseHarnesses.mockReturnValue({
      data: [generic, base, untitled],
    } as ReturnType<typeof useHarnesses>);
  });

  it("shows the selected harness purpose and keeps the trigger to the name", () => {
    render(<HarnessSelect id="harness" value={generic.id} onValueChange={() => {}} />);

    const trigger = screen.getByRole("combobox");
    expect(trigger).toHaveTextContent("Generic");
    expect(trigger).not.toHaveTextContent("Everyday agents");
    expect(trigger).toHaveAttribute("aria-describedby");

    const purpose = document.getElementById(trigger.getAttribute("aria-describedby") ?? "");
    expect(purpose).toHaveTextContent(generic.description ?? "");
  });

  it("lists a purpose under each harness that has one", () => {
    render(<HarnessSelect value={generic.id} onValueChange={() => {}} />);

    fireEvent.click(screen.getByRole("combobox"));

    expect(screen.getByRole("option", { name: /Generic/ })).toHaveTextContent(
      "Everyday agents: files, a shell, the web, and the usual safeguards.",
    );
    expect(screen.getByRole("option", { name: /Base/ })).toHaveTextContent(
      "A blank start. Add only the capabilities this agent should have.",
    );
    const custom = screen.getByRole("option", { name: "Custom" });
    expect(custom).toHaveTextContent("Custom");
    expect(custom).not.toHaveTextContent("Everyday agents");
  });

  it("omits the purpose when nothing with a description is selected", () => {
    render(
      <HarnessSelect
        value=""
        onValueChange={() => {}}
        includeNoneOption
        noneLabel="No parent harness"
      />,
    );

    expect(screen.getByRole("combobox")).toHaveTextContent("No parent harness");
    expect(screen.queryByText(/Everyday agents/)).not.toBeInTheDocument();
  });
});
