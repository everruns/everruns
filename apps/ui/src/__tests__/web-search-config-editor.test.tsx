import { fireEvent, render, screen } from "@testing-library/react";
import { CapabilityEditorContext } from "@/components/agents/capability-editor-context";
import { CapabilitySettingsEditor } from "@/components/agents/capability-settings-editor";
import {
  WebSearchConfigEditor,
  cleanWebSearchConfig,
} from "@/components/agents/web-search-config-editor";
import type { Capability } from "@/lib/api/types";
import { BRAVE_ACCOUNT_ID } from "@/app/dev/_fixtures/web-search-fixtures";

const previewRequests: unknown[] = [];

jest.mock("@/hooks/use-search-providers", () => ({
  useSearchProviders: () => ({
    data: {
      built_in: [],
      native_providers: [],
      connectors: [],
      accounts: [
        {
          id: "00000000-0000-0000-0000-000000000007",
          name: "Team",
          provider: "brave_search",
          status: "ready",
          connected_at: "2026-10-11T00:00:00Z",
        },
      ],
      defaults: { provider: "auto", results_per_search: 8, automatic_order: ["native", "free"] },
    },
  }),
  useSearchPlanPreview: (request: unknown) => {
    previewRequests.push(request);
    return {
      isLoading: false,
      error: null,
      data: {
        provider: "auto",
        fallback: "next",
        limit: 8,
        model_has_native: true,
        org_blocked_domains: ["quora.com"],
        steps: [
          {
            id: "native",
            label: "Model provider search",
            available: true,
            price_note: "Billed by the model provider, about $10 per 1,000 searches",
          },
          { id: "free", label: "Free search", available: true },
        ],
      },
    };
  },
}));

jest.mock("@/hooks/use-organizations", () => ({
  useOrganization: () => ({ data: { default_model_id: "model-1" } }),
}));

jest.mock("@/hooks/use-providers", () => ({
  useModels: () => ({
    data: [
      { id: "model-1", display_name: "GPT-6 Luna" },
      { id: "model-2", display_name: "Claude Sonnet 5.5" },
    ],
  }),
}));

beforeEach(() => previewRequests.splice(0));

describe("cleanWebSearchConfig", () => {
  it("writes only what the author changed", () => {
    expect(cleanWebSearchConfig({ provider: "org_default" })).toEqual({});
    expect(
      cleanWebSearchConfig({
        provider: "account",
        account_id: BRAVE_ACCOUNT_ID,
        allowed_domains: [],
        show_provider: false,
        limit: 5,
      }),
    ).toEqual({ provider: "account", account_id: BRAVE_ACCOUNT_ID, limit: 5 });
    // An account id only travels with the account provider.
    expect(cleanWebSearchConfig({ provider: "free", account_id: BRAVE_ACCOUNT_ID })).toEqual({
      provider: "free",
    });
  });
});

describe("WebSearchConfigEditor", () => {
  it("is the editor for the web_search capability", () => {
    const capability: Capability = {
      id: "web_search",
      name: "Web search",
      description: "Search the web",
      status: "available",
      config_schema: { type: "object" },
    };
    render(<CapabilitySettingsEditor capability={capability} config={{}} onChange={jest.fn()} />);
    expect(screen.getByText("How this agent will search")).toBeInTheDocument();
  });

  it("explains the plan for the agent's model in plain words", () => {
    render(
      <CapabilityEditorContext.Provider value={{ modelId: "model-2" }}>
        <WebSearchConfigEditor config={{}} onChange={jest.fn()} />
      </CapabilityEditorContext.Provider>,
    );
    expect(previewRequests.at(-1)).toEqual({ config: {}, model_id: "model-2" });
    expect(
      screen.getByText(
        "With Claude Sonnet 5.5: Model provider search (billed by the model provider, about $10 per 1,000 searches) answers first.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("The organization also blocks quora.com.")).toBeInTheDocument();
    expect(screen.getByLabelText("Search provider")).toHaveTextContent(
      "Organization default · Automatic",
    );
  });

  it("uses the organization's default model when the agent has none", () => {
    render(<WebSearchConfigEditor config={{}} onChange={jest.fn()} />);
    expect(previewRequests.at(-1)).toEqual({ config: {}, model_id: undefined });
    expect(screen.getByText(/^With GPT-6 Luna:/)).toBeInTheDocument();
  });

  it("picks a named search account", async () => {
    const onChange = jest.fn();
    render(<WebSearchConfigEditor config={{}} onChange={onChange} />);
    fireEvent.click(screen.getByLabelText("Search provider"));
    const option = await screen.findByRole("option", { name: /Brave Search · Team/ });
    fireEvent.pointerDown(option, { pointerType: "mouse" });
    fireEvent.click(option);
    expect(onChange).toHaveBeenLastCalledWith({
      provider: "account",
      account_id: BRAVE_ACCOUNT_ID,
    });
  });

  it("toggles telling the model which provider answered", () => {
    const onChange = jest.fn();
    render(<WebSearchConfigEditor config={{ provider: "free" }} onChange={onChange} />);
    fireEvent.click(screen.getByRole("switch", { name: "Tell the model which provider answered" }));
    expect(onChange).toHaveBeenLastCalledWith({ provider: "free", show_provider: true });
  });
});
