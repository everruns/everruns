// THREAT[TM-WEB-022]: sibling hosts never satisfy the mutation Origin check.
// Hostnames select one existing channel. Never accept an upstream URL, org or
// channel override from a browser. Reserve the broker name from publication.
export function channelFromHost(host: string, domain: string): string | null {
  const suffix = `.${domain}`;
  if (!host.endsWith(suffix)) return null;
  const label = host.slice(0, -suffix.length);
  return /^[0-9a-f]{32}$/.test(label) ? `appchan_${label}` : null;
}
export function settings() {
  const required = (key: string) => {
    const value = process.env[key];
    if (!value) throw new Error(`Missing ${key}`);
    return value;
  };
  const domain = required("CHAT_DOMAIN");
  if (!/^[a-z0-9.-]+$/.test(domain) || domain.includes(".."))
    throw new Error("Invalid CHAT_DOMAIN");
  const issuer = required("CHAT_OIDC_ISSUER");
  if (new URL(issuer).protocol !== "https:") throw new Error("HTTPS issuer required");
  return {
    domain,
    broker: `https://login.${domain}`,
    issuer,
    clientId: required("CHAT_OIDC_CLIENT_ID"),
    clientSecret: required("CHAT_OIDC_CLIENT_SECRET"),
    key: required("CHAT_SESSION_KEY"),
    database: required("CHAT_DATABASE_URL"),
    upstream: required("CHAT_API_URL").replace(/\/$/, ""),
  };
}
export function hostOf(request: Request) {
  // Next's internal URL can name the proxy listener. The edge preserves Host;
  // never select a channel from a client-supplied X-Forwarded-Host.
  const host = request.headers.get("host");
  if (!host) return new URL(request.url).hostname.toLowerCase();
  const url = new URL(`https://${host}`);
  if (url.username || url.password || url.pathname !== "/" || url.search || url.hash)
    throw new Error("Invalid host");
  return url.hostname.toLowerCase();
}
export function channelOf(request: Request) {
  const channel = channelFromHost(hostOf(request), settings().domain);
  if (!channel) throw new Error("Unknown chat host");
  return channel;
}
export function safeMutation(request: Request) {
  const origin = request.headers.get("origin");
  return origin === `https://${hostOf(request)}`;
}
export async function bootstrap(channel: string) {
  const response = await fetch(`${settings().upstream}/v1/channels/${channel}/chat/config`, {
    cache: "no-store",
    signal: AbortSignal.timeout(10000),
  });
  if (!response.ok) throw new Error("Chat unavailable");
  return response.json();
}
