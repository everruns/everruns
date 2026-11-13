import { tracingAuthHeaders, tracingProviders } from "@/components/tracing/provider-config";

it("offers explicit native receiver endpoints", () => {
  expect(tracingProviders.langfuse.endpoint).toBe(
    "https://cloud.langfuse.com/api/public/otel/v1/traces",
  );
  expect(tracingProviders.langsmith.endpoint).toBe(
    "https://api.smith.langchain.com/otel/v1/traces",
  );
});
it("builds Langfuse Basic authentication without storing keys in config", () => {
  expect(tracingAuthHeaders("langfuse", " sk-example ", " pk-example ", "X-API-Key")).toEqual({
    Authorization: `Basic ${btoa("pk-example:sk-example")}`,
  });
});
it("uses LangSmith API key header irrespective of a previous provider selection", () => {
  expect(tracingAuthHeaders("langsmith", " ls-example ", "", "Authorization")).toEqual({
    "x-api-key": "ls-example",
  });
});
it("preserves Braintrust Bearer handling and generic OTLP headers", () => {
  expect(tracingAuthHeaders("braintrust", " Bearer bt-example ", "", "Api-Key")).toEqual({
    Authorization: "Bearer bt-example",
  });
  expect(tracingAuthHeaders("otlp", "Bearer ot-example", "", "Authorization")).toEqual({
    Authorization: "Bearer ot-example",
  });
  expect(tracingAuthHeaders("otlp", "", "", "Authorization")).toEqual({});
});
