import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type React from "react";
import { EditModelDialog } from "@/components/models/edit-model-dialog";
import type { LlmModelWithProvider, LlmProvider } from "@/lib/api/types";

jest.mock("@/components/ui/select", () => {
  const React = jest.requireActual("react");
  const SelectContext = React.createContext({ onValueChange: (_value: string) => {} });

  return {
    Select: ({
      children,
      onValueChange,
    }: {
      children: React.ReactNode;
      onValueChange: (value: string) => void;
    }) => <SelectContext.Provider value={{ onValueChange }}>{children}</SelectContext.Provider>,
    SelectTrigger: ({ children, id }: { children: React.ReactNode; id?: string }) => (
      <div id={id}>{children}</div>
    ),
    SelectValue: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
    SelectContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
    SelectItem: ({ value, children }: { value: string; children: React.ReactNode }) => {
      const { onValueChange } = React.useContext(SelectContext);
      return (
        <button type="button" onClick={() => onValueChange(value)}>
          {children}
        </button>
      );
    },
  };
});

const providers: LlmProvider[] = [
  {
    id: "provider-1",
    name: "OpenAI Production",
    provider_type: "openai",
    status: "active",
    api_key_set: true,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
  {
    id: "provider-2",
    name: "OpenRouter",
    provider_type: "openai",
    status: "active",
    api_key_set: true,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
];

const model: LlmModelWithProvider = {
  id: "model-1",
  provider_id: "provider-1",
  model_id: "gpt-4.5",
  display_name: "GPT-4.5",
  provider_name: "OpenAI Production",
  provider_type: "openai",
  healthy: true,
  capabilities: [],
  enabled: true,
  is_favorite: false,
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
};

describe("EditModelDialog", () => {
  it("submits provider changes with the model update", async () => {
    const onSubmit = jest.fn().mockResolvedValue(undefined);

    render(
      <EditModelDialog
        model={model}
        providers={providers}
        open
        onOpenChange={jest.fn()}
        onSubmit={onSubmit}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "OpenRouter" }));
    fireEvent.change(screen.getByLabelText("Display Name"), {
      target: { value: "GPT-4.5 via OpenRouter" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save Changes" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith("model-1", {
        provider_id: "provider-2",
        model_id: "gpt-4.5",
        display_name: "GPT-4.5 via OpenRouter",
        enabled: true,
      });
    });
  });
});
