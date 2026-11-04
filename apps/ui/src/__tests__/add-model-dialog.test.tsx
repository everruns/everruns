import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import * as mockReact from "react";
import { AddModelDialog } from "@/components/models/add-model-dialog";
import type { ModelProfileResponse, Provider } from "@/lib/api/types";

let mockProfiles: ModelProfileResponse[] = [];
const mutateAsync = jest.fn().mockResolvedValue({});

jest.mock("@/hooks/use-providers", () => ({
  useProvidersConfig: () => ({ data: undefined }),
  useModelProfiles: () => ({ data: mockProfiles }),
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
  beforeEach(() => {
    mutateAsync.mockClear();
    mockProfiles = [];
  });

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
        service: "embeddings",
        profile_key: undefined,
        enabled: true,
      }),
    );
  });
});

describe("Jev profile wire IDs", () => {
  it.each([
    ["typesafe", "jev-1.14.0", "jev-1.14.0"],
    ["openrouter", "jev-1.14.0", "typesafe/jev-1.14"],
    ["openrouter", "jev-1.13.0", "typesafe/jev-1.13"],
    ["openrouter", "jev-latest", "~typesafe/jev-latest"],
  ])("saves %s %s as %s", async (providerType, canonical, wireId) => {
    mutateAsync.mockClear();
    mockProfiles = [
      {
        key: `typesafe/${canonical}`,
        service: "decisions",
        source: "curated",
        profile: { name: "Jev", family: "jev" } as ModelProfileResponse["profile"],
      },
    ];
    render(
      <AddModelDialog
        providers={[{ ...providers[0], provider_type: providerType as Provider["provider_type"] }]}
        open
        onOpenChange={jest.fn()}
      />,
    );
    fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "provider_openai" } });
    fireEvent.change(screen.getByLabelText("Model type"), { target: { value: "decisions" } });
    fireEvent.change(screen.getByLabelText("Model profile"), {
      target: { value: `typesafe/${canonical}` },
    });
    expect(screen.getByLabelText("Model ID")).toHaveValue(wireId);
    fireEvent.click(screen.getByRole("button", { name: "Create Model" }));
    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith(
        expect.objectContaining({
          model_id: wireId,
          service: "decisions",
          profile_key: `typesafe/${canonical}`,
        }),
      ),
    );
  });
});
