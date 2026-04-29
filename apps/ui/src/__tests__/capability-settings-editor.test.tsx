import { render, screen } from "@testing-library/react";
import {
  CapabilitySettingsEditor,
  hasCapabilitySettings,
} from "@/components/agents/capability-settings-editor";
import type { Capability } from "@/lib/api/types";

function capability(overrides: Partial<Capability>): Capability {
  return {
    id: overrides.id ?? "test_capability",
    name: overrides.name ?? "Test Capability",
    description: overrides.description ?? "Test capability",
    status: overrides.status ?? "available",
    ...overrides,
  };
}

function imageGenerationCapability(): Capability {
  return capability({
    id: "gpt_image_gen",
    config_schema: {
      type: "object",
      properties: {
        model: {
          type: "string",
          title: "Image Model",
          description: "Default model used by image generation and edit tools.",
          default: "gpt-image-2",
          oneOf: [
            { const: "gpt-image-2", title: "ChatGPT Images 2.0" },
            { const: "gpt-image-1", title: "GPT Image 1" },
          ],
        },
        default_quality: {
          type: "string",
          title: "Default Quality",
          description: "Default quality used when a tool call does not specify quality.",
          default: "medium",
          oneOf: [
            { const: "medium", title: "Medium" },
            { const: "low", title: "Low" },
            { const: "high", title: "High" },
            { const: "auto", title: "Auto" },
          ],
        },
        partial_images: {
          type: "integer",
          title: "Progress Updates",
          description: "Number of streamed in-progress image updates for single-image requests.",
          default: 1,
          minimum: 0,
          maximum: 3,
          oneOf: [
            { const: 1, title: "1 Progress Update" },
            { const: 0, title: "Off" },
            { const: 2, title: "2 Progress Updates" },
            { const: 3, title: "3 Progress Updates" },
          ],
        },
      },
    },
    config_ui_schema: {
      "ui:order": ["model", "default_quality", "partial_images"],
      partial_images: { "ui:widget": "select" },
    },
  });
}

function parallelCapability(): Capability {
  return capability({
    id: "parallel_search",
    config_schema: {
      type: "object",
      properties: {
        auth: {
          type: "string",
          title: "Authentication",
          description: "Free works without setup. API key uses Settings > Connections > Parallel.",
          default: "free",
          oneOf: [
            { const: "free", title: "Free" },
            { const: "connection", title: "Parallel API Key" },
          ],
        },
        endpoint: {
          type: "string",
          title: "MCP Endpoint",
          description:
            "OAuth MCP uses https://search.parallel.ai/mcp-oauth and requires a Parallel API key.",
          default: "free",
          oneOf: [
            { const: "free", title: "Free MCP" },
            { const: "oauth", title: "OAuth MCP" },
          ],
        },
      },
    },
    config_ui_schema: {
      "ui:order": ["auth", "endpoint"],
      auth: { "ui:widget": "select" },
      endpoint: { "ui:widget": "select" },
    },
  });
}

describe("CapabilitySettingsEditor", () => {
  it("exposes settings when a capability provides config schema", () => {
    expect(hasCapabilitySettings(capability({ id: "gpt_image_gen" }))).toBe(false);
    expect(
      hasCapabilitySettings(
        capability({
          id: "schema_capability",
          config_schema: { type: "object", properties: { enabled: { type: "boolean" } } },
        }),
      ),
    ).toBe(true);
  });

  it("renders OpenAI image generation settings from schema metadata", () => {
    render(
      <CapabilitySettingsEditor
        capability={imageGenerationCapability()}
        config={{}}
        onChange={jest.fn()}
      />,
    );

    expect(screen.getByText("Image Model")).toBeInTheDocument();
    expect(screen.getByText("ChatGPT Images 2.0")).toBeInTheDocument();
    expect(screen.getByText(/Default model used by image generation/i)).toBeInTheDocument();
    expect(screen.getByText("Default Quality")).toBeInTheDocument();
    expect(screen.getByText("Medium")).toBeInTheDocument();
    expect(screen.getByText(/Default quality used when a tool call/i)).toBeInTheDocument();
    expect(screen.getByText("Progress Updates")).toBeInTheDocument();
    expect(screen.getByText("1 Progress Update")).toBeInTheDocument();
    expect(screen.getByText(/Number of streamed in-progress image updates/i)).toBeInTheDocument();
  });

  it("renders schema-backed text settings", () => {
    render(
      <CapabilitySettingsEditor
        capability={capability({
          id: "schema_capability",
          config_schema: {
            type: "object",
            properties: {
              endpoint: {
                type: "string",
                title: "Endpoint URL",
                description: "Base URL used by the capability.",
              },
            },
          },
        })}
        config={{ endpoint: "https://example.com" }}
        onChange={jest.fn()}
      />,
    );

    expect(screen.getByLabelText("Endpoint URL")).toHaveValue("https://example.com");
    expect(screen.getByText("Base URL used by the capability.")).toBeInTheDocument();
  });

  it("renders Parallel settings from schema metadata", () => {
    render(
      <CapabilitySettingsEditor
        capability={parallelCapability()}
        config={{}}
        onChange={jest.fn()}
      />,
    );

    expect(screen.getByText("Authentication")).toBeInTheDocument();
    expect(screen.getByText("Free")).toBeInTheDocument();
    expect(screen.getByText("MCP Endpoint")).toBeInTheDocument();
    expect(screen.getByText("Free MCP")).toBeInTheDocument();
    expect(screen.getByText(/works without setup/i)).toBeInTheDocument();
  });
});
