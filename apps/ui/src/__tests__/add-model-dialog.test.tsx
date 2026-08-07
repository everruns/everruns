import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import * as mockReact from "react";
import { AddModelDialog } from "@/components/models/add-model-dialog";
import type { Provider } from "@/lib/api/types";

const mutateAsync = jest.fn().mockResolvedValue({});

jest.mock("@/hooks/use-providers", () => ({
  useCreateModel: () => ({ mutateAsync, isPending: false }),
}));

jest.mock("@/components/ui/select", () => {
  function collectOptions(node: mockReact.ReactNode): mockReact.ReactNode[] {
    const options: mockReact.ReactNode[] = [];
    mockReact.Children.forEach(node, (child: mockReact.ReactNode) => {
      if (!mockReact.isValidElement(child)) return;
      if ((child.type as { isSelectItem?: boolean }).isSelectItem) {
        const props = child.props as { value: string; children: mockReact.ReactNode };
        options.push(
          <option key={props.value} value={props.value}>
            {props.children}
          </option>,
        );
        return;
      }
      options.push(...collectOptions((child.props as { children?: mockReact.ReactNode }).children));
    });
    return options;
  }

  function findTrigger(node: mockReact.ReactNode): { id?: string } {
    let found: { id?: string } = {};
    mockReact.Children.forEach(node, (child: mockReact.ReactNode) => {
      if (!mockReact.isValidElement(child) || found.id) return;
      if ((child.type as { isSelectTrigger?: boolean }).isSelectTrigger) {
        found = { id: (child.props as { id?: string }).id };
        return;
      }
      const nested = findTrigger((child.props as { children?: mockReact.ReactNode }).children);
      if (nested.id) found = nested;
    });
    return found;
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
    return (
      <select
        id={findTrigger(children).id}
        value={value}
        onChange={(event) => onValueChange(event.target.value)}
      >
        {collectOptions(children)}
      </select>
    );
  }

  function SelectItem(_: { value: string; children: mockReact.ReactNode }) {
    return null;
  }
  SelectItem.isSelectItem = true;

  function SelectTrigger(_: { children: mockReact.ReactNode; id?: string }) {
    return null;
  }
  SelectTrigger.isSelectTrigger = true;

  return {
    Select,
    SelectContent: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectItem,
    SelectTrigger,
    SelectValue: () => null,
  };
});

const providers: Provider[] = [
  {
    id: "provider_openai",
    name: "OpenAI",
    provider_type: "openai",
    api_key_set: true,
    status: "active",
    managed: false,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  },
];

describe("AddModelDialog", () => {
  beforeEach(() => mutateAsync.mockClear());

  it("creates a selectable embedding model", async () => {
    render(<AddModelDialog providers={providers} open onOpenChange={jest.fn()} />);

    fireEvent.change(screen.getByLabelText("Provider"), {
      target: { value: "provider_openai" },
    });
    fireEvent.change(screen.getByLabelText("Model type"), {
      target: { value: "embeddings" },
    });
    fireEvent.change(screen.getByLabelText("Model ID"), {
      target: { value: "text-embedding-3-small" },
    });
    fireEvent.change(screen.getByLabelText("Display Name"), {
      target: { value: "Text Embedding 3 Small" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Model" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        model_id: "text-embedding-3-small",
        display_name: "Text Embedding 3 Small",
        capabilities: ["embeddings"],
        enabled: true,
      }),
    );
  });
});
