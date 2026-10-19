// The object owns endpoint/credential policy; the shared Rust driver owns HTTP
// payloads, provider protocol parsing, retry classification and usage metadata.
export async function fetchModel(env, raw, signal, fetcher = fetch) {
  const url = new URL(
    `${(env.OPENAI_BASE_URL ?? "https://api.openai.com/v1").replace(/\/$/, "")}/chat/completions`,
  );
  if (
    !(
      url.protocol === "https:" ||
      (url.protocol === "http:" && ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname))
    ) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  )
    throw new Error("invalid provider endpoint");
  if (!env.OPENAI_API_KEY) throw new Error("provider credential missing");
  const request = JSON.parse(raw);
  if (request.method !== "POST" || request.path !== "/chat/completions")
    throw new Error("unsupported provider request");
  const headers = new Headers(request.headers);
  headers.set("authorization", `Bearer ${env.OPENAI_API_KEY}`);
  return fetcher(url, {
    method: "POST",
    redirect: "manual",
    signal: AbortSignal.any([signal, AbortSignal.timeout(60000)]),
    headers,
    body: request.body,
  });
}
