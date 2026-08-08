import { getLocalizedOutputMessageText, localizeRuntimeError } from "@/lib/runtime-errors";

describe("runtime error localization", () => {
  it("localizes output message text from structured budget metadata instead of fallback prose", () => {
    const text = getLocalizedOutputMessageText("uk", {
      message: {
        id: "msg-1",
        session_id: "session-1",
        sequence: 1,
        role: "agent",
        content: [{ type: "text", text: "backend fallback changed" }],
        metadata: {
          error_code: "budget_exhausted",
          error_fields: { spent: 12.5, limit: 10, currency: "usd" },
        },
        tool_call_id: null,
        created_at: "2025-01-01T00:00:00.000Z",
      },
      error_code: "budget_exhausted",
      error_fields: { spent: 12.5, limit: 10, currency: "usd" },
    });

    expect(text).toContain("Бюджет вичерпано");
    expect(text).toContain("12,50");
    expect(text).toContain("10,00");
  });

  it("falls back to structured provider rate-limit copy when retry_after is present", () => {
    const text = localizeRuntimeError(
      "uk",
      {
        code: "provider_rate_limited",
        fields: { retry_after: 9 },
      },
      "backend fallback changed",
    );

    expect(text).toContain("9 с");
    expect(text).not.toContain("backend fallback changed");
  });

  it("localizes provider quota exhaustion distinctly from misconfiguration", () => {
    const text = localizeRuntimeError(
      "en",
      { code: "provider_quota_exhausted", fields: { provider: "openai" } },
      "backend fallback",
    );

    expect(text).toContain("out of credits or quota");
    expect(text).not.toContain("misconfiguration");
  });

  it("appends the detail field from detailed disclosure mode to localized copy", () => {
    const text = localizeRuntimeError(
      "en",
      {
        code: "provider_quota_exhausted",
        fields: { detail: "OpenAI API error (429): insufficient_quota" },
      },
      "backend fallback",
    );

    expect(text).toContain("out of credits or quota");
    expect(text).toContain("Details: OpenAI API error (429): insufficient_quota");
  });

  it("localizes the detail label for non-English locales", () => {
    const text = localizeRuntimeError(
      "uk",
      {
        code: "provider_quota_exhausted",
        fields: { detail: "OpenAI API error (429): insufficient_quota" },
      },
      "backend fallback",
    );

    expect(text).toContain("кредити або квота");
    expect(text).toContain("Деталі: OpenAI API error (429): insufficient_quota");
    expect(text).not.toContain("Details:");
  });

  it("renders invalid tool schemas as an actionable integration error", () => {
    const text = localizeRuntimeError(
      "en",
      {
        code: "invalid_tool_schema",
        fields: { provider: "openai", schema_path: "$.properties.email.pattern" },
      },
      "generic processing error",
    );

    expect(text).toContain("connected tool");
    expect(text).toContain("Update the integration");
    expect(text).not.toContain("generic processing error");
  });
});
