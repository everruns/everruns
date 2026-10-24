import {
  isPublicDraftLocation,
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
