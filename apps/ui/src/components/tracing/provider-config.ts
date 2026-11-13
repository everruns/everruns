import type { TracingConfig } from "@/lib/api/tracing";

export const tracingProviders = {
  braintrust: { label: "Braintrust", endpoint: "https://api.braintrust.dev" },
  langfuse: { label: "Langfuse", endpoint: "https://cloud.langfuse.com/api/public/otel/v1/traces" },
  langsmith: { label: "LangSmith", endpoint: "https://api.smith.langchain.com/otel/v1/traces" },
  otlp: { label: "OpenTelemetry / OTLP", endpoint: "" },
} satisfies Record<TracingConfig["provider"], { label: string; endpoint: string }>;

/** Build only write-only headers. Never copy credentials into destination configuration. */
export function tracingAuthHeaders(
  provider: TracingConfig["provider"],
  secret: string,
  publicKey: string,
  header: string,
): Record<string, string> {
  switch (provider) {
    case "braintrust":
      return { Authorization: `Bearer ${secret.trim().replace(/^Bearer\s+/i, "")}` };
    case "langfuse":
      return { Authorization: `Basic ${btoa(`${publicKey.trim()}:${secret.trim()}`)}` };
    case "langsmith":
      return { "x-api-key": secret.trim() };
    case "otlp":
      return secret.trim() ? { [header]: secret } : {};
  }
}
