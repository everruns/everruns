// Endpoint and credential policy only; the shared OpenRouter driver owns JSON/SSE.
export async function fetchModel(env, raw, signal, fetcher = fetch) {
  const request = JSON.parse(raw);
  if (request.method !== "POST" || request.path !== "/responses")
    throw new Error("unsupported provider request");
  const url = new URL(
    `${(env.OPENROUTER_BASE_URL ?? "https://openrouter.ai/api/v1").replace(/\/$/, "")}${request.path}`,
  );
  if (
    !(
      url.protocol === "https:" ||
      (url.protocol === "http:" &&
        ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname))
    ) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  )
    throw new Error("invalid provider endpoint");
  if (!env.OPENROUTER_API_KEY) throw new Error("provider credential missing");
  const headers = new Headers(request.headers);
  headers.set("authorization", `Bearer ${env.OPENROUTER_API_KEY}`);
  return fetcher(url, {
    method: "POST",
    redirect: "manual",
    signal: AbortSignal.any([signal, AbortSignal.timeout(60000)]),
    headers,
    body: request.body,
  });
}
