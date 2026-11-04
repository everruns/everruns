// One call to the OpenAI Responses API, plus the `llm.generation` trace
// event Everruns shows for it. The model key belongs to this Worker;
// Everruns never receives it, and provider error text is never
// forwarded.
import { runtimeMessage } from "./protocol.js";

export const MODEL = "gpt-6.1-sol";
const RESPONSES_URL = "https://api.openai.com/v1/responses";
const TIMEOUT_MS = 60_000;

export async function callModel({
  messages,
  tools,
  env,
  emit,
  span,
  modelFetch,
  signal,
}) {
  const startedAt = Date.now();
  // Failed calls are traced too, so they show up in Trace.
  const trace = (metadata, output = { text: null, tool_calls: [] }) =>
    emit(
      "llm.generation",
      {
        messages: messages.filter((m) => m.role).map(toTraceMessage),
        output,
        metadata: {
          model: MODEL,
          provider: "openai",
          duration_ms: Date.now() - startedAt,
          ...metadata,
        },
      },
      span,
    );

  let response;
  try {
    response = await modelFetch(RESPONSES_URL, {
      method: "POST",
      redirect: "manual",
      headers: {
        Authorization: `Bearer ${env.OPENAI_API_KEY}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        model: MODEL,
        reasoning: { effort: "low" },
        input: messages,
        tools,
        max_output_tokens: 4096,
        store: false,
        include: ["reasoning.encrypted_content"],
      }),
      signal: withTimeout(signal),
    });
  } catch {
    await trace({
      success: false,
      error: "Model transport failed",
      finish_reasons: ["error"],
    });
    throw new Error("Model request failed");
  }

  if (!response.ok) {
    const code = await safeErrorCode(response);
    const suffix = code ? ` (${code})` : "";
    const error = `Model HTTP ${response.status}${suffix}`;
    await trace({ success: false, error, finish_reasons: ["error"] });
    console.error("Model request failed", {
      status: response.status,
      code,
    });
    throw new Error("Model request failed");
  }

  const result = await response.json();
  if (result.status !== "completed" || !Array.isArray(result.output)) {
    throw new Error("Incomplete model response");
  }

  const text = result.output
    .filter((item) => item.type === "message")
    .flatMap((item) => item.content ?? [])
    .filter((part) => part.type === "output_text")
    .map((part) => part.text)
    .join("");
  const toolCalls = result.output
    .filter((item) => item.type === "function_call")
    .map((call) => ({
      id: call.call_id,
      name: call.name,
      arguments: JSON.parse(call.arguments),
    }));
  const cached = result.usage?.input_tokens_details?.cached_tokens ?? 0;
  const usage = {
    input_tokens: (result.usage?.input_tokens ?? 0) - cached,
    output_tokens: result.usage?.output_tokens ?? 0,
    cache_read_tokens: cached,
  };

  await trace(
    {
      response_model: result.model,
      usage,
      success: true,
      finish_reasons: [toolCalls.length ? "tool_calls" : "stop"],
    },
    { text: text || null, tool_calls: toolCalls },
  );
  return { output: result.output, text, toolCalls, usage };
}

function withTimeout(signal) {
  const timeout = AbortSignal.timeout(TIMEOUT_MS);
  return signal ? AbortSignal.any([signal, timeout]) : timeout;
}

// Only a short machine-readable code is kept; the provider's message
// may echo request content.
async function safeErrorCode(response) {
  const diagnostic = await response.json().catch(() => ({}));
  const code = diagnostic.error?.code;
  return typeof code === "string" && /^[a-z0-9_]{1,64}$/.test(code)
    ? code
    : null;
}

function toTraceMessage(message) {
  const role = message.role === "assistant" ? "agent" : message.role;
  const text =
    typeof message.content === "string"
      ? message.content
      : (message.content ?? [])
          .filter(
            (p) => p.type === "output_text" || p.type === "input_text",
          )
          .map((p) => p.text)
          .join("");
  return runtimeMessage(role, text);
}
