import {
  a2aSamples,
  apiBaseUrl,
  codingAgentPrompt,
  openApiUrl,
  sdkSessionSamples,
  sessionFlowSamples,
  webhookSamples,
} from "@/lib/integration/snippets";

const ORIGIN = "https://app.example.com";

describe("integration snippets", () => {
  it("derives REST and OpenAPI URLs from the origin", () => {
    expect(apiBaseUrl(ORIGIN)).toBe("https://app.example.com/api/v1");
    expect(openApiUrl(ORIGIN)).toBe("https://app.example.com/api-doc/openapi.json");
  });

  it("falls back to a placeholder host when origin is empty", () => {
    expect(apiBaseUrl("")).toContain("your-everruns-host");
  });

  it("binds the session flow to an agent across all four languages", () => {
    const samples = sessionFlowSamples({ origin: ORIGIN, agentId: "agent_123" });
    expect(samples.map((s) => s.label)).toEqual(["curl", "TypeScript", "Python", "Rust"]);
    for (const sample of samples) {
      expect(sample.code).toContain("agent_123");
      expect(sample.code).not.toContain("harness_id");
    }
  });

  it("binds the session flow to a harness when given a harness id", () => {
    const samples = sessionFlowSamples({ origin: ORIGIN, harnessId: "harness_9" });
    for (const sample of samples) {
      expect(sample.code).toContain("harness_id");
      expect(sample.code).toContain("harness_9");
    }
  });

  it("does not leak unresolved template interpolation into generated code", () => {
    const all = [
      ...sessionFlowSamples({ origin: ORIGIN, agentId: "a" }),
      ...sdkSessionSamples({ origin: ORIGIN, agentId: "a" }),
      ...webhookSamples({ origin: ORIGIN, endpointUrl: "/api/v1/apps/x/webhooks/y" }),
      ...a2aSamples({ origin: ORIGIN, endpointUrl: "https://h/api/v1/apps/x/a2a/y" }),
    ];
    for (const sample of all) {
      expect(sample.code).not.toContain("${");
    }
  });

  it("emits SDK install hints and the /api base for the SDK samples", () => {
    const samples = sdkSessionSamples({ origin: ORIGIN, agentId: "agent_123" });
    expect(samples.map((s) => s.label)).toEqual(["Python", "TypeScript", "Rust"]);
    expect(samples.find((s) => s.label === "Python")?.code).toContain("pip install everruns-sdk");
    expect(samples.find((s) => s.label === "Rust")?.code).toContain("cargo add everruns-sdk");
    expect(samples.find((s) => s.label === "TypeScript")?.code).toContain(
      "npm install @everruns/sdk",
    );
    // SDK base URL is the /api root, not /api/v1.
    expect(samples.find((s) => s.label === "Python")?.code).toContain(`${ORIGIN}/api"`);
  });

  it("absolutizes relative webhook endpoints", () => {
    const [curl] = webhookSamples({ origin: ORIGIN, endpointUrl: "/api/v1/apps/x/webhooks/y" });
    expect(curl.code).toContain("https://app.example.com/api/v1/apps/x/webhooks/y");
  });

  it("builds a coding-agent prompt that references the entity, flow, and SDKs", () => {
    const prompt = codingAgentPrompt({
      origin: ORIGIN,
      kind: "agent",
      id: "agent_123",
      name: "Support Bot",
    });
    expect(prompt).toContain("agent_123");
    expect(prompt).toContain("Support Bot");
    expect(prompt).toContain("/sessions");
    expect(prompt).toContain("everruns-sdk");
    expect(prompt).toContain(openApiUrl(ORIGIN));
  });

  it("opens the coding-agent prompt with a subject matching the kind", () => {
    expect(codingAgentPrompt({ origin: ORIGIN, kind: "agent", id: "a" })).toMatch(
      /^I want to integrate an Everruns agent /,
    );
    expect(codingAgentPrompt({ origin: ORIGIN, kind: "harness", id: "h" })).toMatch(
      /^I want to integrate an Everruns harness /,
    );
    expect(codingAgentPrompt({ origin: ORIGIN, kind: "webhook", endpointUrl: "/w" })).toMatch(
      /^I want to integrate an Everruns app \(webhook channel\) /,
    );
    expect(codingAgentPrompt({ origin: ORIGIN, kind: "a2a", endpointUrl: "/a" })).toMatch(
      /^I want to integrate an Everruns app \(A2A channel\) /,
    );
  });
});
