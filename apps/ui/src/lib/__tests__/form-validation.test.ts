import {
  agentFormSchema,
  agentIdentityFormSchema,
  apiKeySecretSchema,
  createConnectionFormSchema,
  harnessFormSchema,
  mcpServerFormSchema,
  parseTagList,
} from "@/lib/form-validation";

describe("form validation schemas", () => {
  it("trims and normalizes agent form values", () => {
    const parsed = agentFormSchema.parse({
      name: " customer-support ",
      display_name: " Customer Support ",
      description: " Handles tickets ",
      system_prompt: " Be concise ",
      default_model_id: " gpt-5.4 ",
    });

    expect(parsed).toEqual({
      name: "customer-support",
      display_name: "Customer Support",
      description: "Handles tickets",
      system_prompt: "Be concise",
      default_model_id: "gpt-5.4",
      tags: "",
    });
  });

  it("rejects invalid agent names and missing prompts", () => {
    const parsed = agentFormSchema.safeParse({
      name: "Customer Support",
      display_name: "",
      description: "",
      system_prompt: " ",
      default_model_id: "",
    });

    expect(parsed.success).toBe(false);
    if (parsed.success) return;

    const messages = parsed.error.issues.map((issue) => issue.message);
    expect(messages).toContain("Name must contain only lowercase letters, numbers, and hyphens");
    expect(messages).toContain("System prompt is required");
  });

  it("parses harness tags into a clean list", () => {
    const parsed = harnessFormSchema.parse({
      name: "ops-harness",
      display_name: "Ops Harness",
      description: "",
      system_prompt: "Do the work",
      parent_harness_id: "",
      default_model_id: "",
      tags: "ops,  alerts , , incident-response ",
    });

    expect(parseTagList(parsed.tags)).toEqual(["ops", "alerts", "incident-response"]);
  });

  it("rejects invalid agent identity locale and timezone values", () => {
    const parsed = agentIdentityFormSchema.safeParse({
      name: "Ops Bot",
      description: "",
      locale: "en-MARS",
      timezone: "Mars/Olympus_Mons",
    });

    expect(parsed.success).toBe(false);
    if (parsed.success) return;

    const messages = parsed.error.issues.map((issue) => issue.message);
    expect(messages).toContain("Select a valid locale");
    expect(messages).toContain("Select a valid timezone");
  });

  it("requires a valid MCP server URL and API key when auth uses api_key", () => {
    const parsed = mcpServerFormSchema.safeParse({
      name: "atlassian",
      description: "",
      url: "not-a-url",
      auth_mode: "api_key",
      api_key: " ",
    });

    expect(parsed.success).toBe(false);
    if (parsed.success) return;

    const messages = parsed.error.issues.map((issue) => issue.message);
    expect(messages).toContain("URL must be a valid absolute URL");
    expect(messages).toContain("API key is required");
  });

  it("validates dynamic provider forms from the connection schema", () => {
    const schema = createConnectionFormSchema([
      { name: "api_key", label: "API Key", field_type: "password", required: true },
      { name: "project", label: "Project", field_type: "text", required: true },
      { name: "region", label: "Region", field_type: "text", required: false },
    ]);

    const parsed = schema.safeParse({
      api_key: " secret ",
      project: " ",
      region: " us-east-1 ",
    });

    expect(parsed.success).toBe(false);
    if (parsed.success) return;

    expect(parsed.error.issues.map((issue) => issue.message)).toContain("Project is required");
    expect(apiKeySecretSchema.parse({ api_key: " secret " }).api_key).toBe("secret");
  });

  it("allows optional provider fields to be omitted entirely", () => {
    const schema = createConnectionFormSchema([
      { name: "api_key", label: "API Key", field_type: "password", required: true },
      { name: "region", label: "Region", field_type: "text", required: false },
    ]);

    const parsed = schema.parse({ api_key: " secret " });

    expect(parsed).toEqual({
      api_key: "secret",
      region: undefined,
    });
  });
});
