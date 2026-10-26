import {
  isPublicDraftLocation,
  scheduleCreateDraftSchema,
  mcpMetadataDraftSchema,
  providerTraceDraftSchema,
} from "@/components/chat-mode/page-form-schemas";

it.each([
  "https://user:fixture-secret@example.test/api",
  "https://example.test/api?token=fixture",
  "https://example.test/api#fixture-secret",
  "https://example.test/api?id=fixture-secret",
  "file:///etc/passwd",
])("keeps credential-bearing or non-public locations out of form reads: %s", (value) => {
  expect(isPublicDraftLocation(value)).toBe(false);
  expect(
    providerTraceDraftSchema.safeParse({
      enabled: true,
      generation_url_template: value,
      session_url_template: "",
    }).success,
  ).toBe(false);
});

it("accepts ordinary endpoints and supported trace substitutions", () => {
  expect(isPublicDraftLocation("https://example.test/api")).toBe(true);
  expect(isPublicDraftLocation("https://example.test/logs?id={response_id}")).toBe(true);
  expect(isPublicDraftLocation("")).toBe(true);
});

it("keeps payloads and credential fields outside secondary metadata schemas", () => {
  const schedule = {
    name: "Daily review",
    description: "",
    cron_expression: "0 0 9 * * * *",
    target_type: "workflow",
    target_name: "review",
    enabled: false,
  };
  expect(scheduleCreateDraftSchema.safeParse(schedule).success).toBe(true);
  expect(
    scheduleCreateDraftSchema.safeParse({ ...schedule, target_input: "payload-only-fixture" })
      .success,
  ).toBe(false);
  const mcp = {
    name: "Tools",
    description: "",
    url: "https://example.test/mcp",
    protocol_mode: "auto",
    elicitation_policy: "url",
    tool_timeout: "",
  };
  expect(mcpMetadataDraftSchema.safeParse(mcp).success).toBe(true);
  expect(
    mcpMetadataDraftSchema.safeParse({ ...mcp, api_key: "browser-only-fixture" }).success,
  ).toBe(false);
  expect(
    mcpMetadataDraftSchema.safeParse({
      ...mcp,
      url: "https://example.test/mcp?token=browser-only-fixture",
    }).success,
  ).toBe(false);
});
