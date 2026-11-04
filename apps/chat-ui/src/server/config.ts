// THREAT[TM-WEB-022]: sibling hosts never satisfy the mutation Origin check.
// The hostname's label selects one existing Work Chat. That label is assigned
// by the API and is not the channel id. Never accept an upstream URL, org, or
// channel id from a browser. Reserve the broker name from publication.
export const HOST_LABEL = /^[abcdefghijkmnpqrstuvwxyz23456789]{10}$/;

/** The single DNS label of a hosted chat. A channel id is not a label. */
export function hostLabel(host: string, domain: string): string | null {
  const suffix = `.${domain.toLowerCase()}`;
  const name = host.toLowerCase();
  if (!name.endsWith(suffix)) return null;
  const label = name.slice(0, -suffix.length);
  return HOST_LABEL.test(label) ? label : null;
}

const resolved = new Map<string, { id: string; at: number }>();

/** The channel published at this hostname. The browser does not choose the id. */
export async function channelForHost(host: string): Promise<string> {
  const label = hostLabel(host, settings().domain);
  if (!label) throw new Error("Unknown chat host");
  const cached = resolved.get(label);
  if (cached && Date.now() - cached.at < 60_000) return cached.id;
  const response = await fetch(`${settings().upstream}/v1/work-chat/hosts/${label}`, {
    cache: "no-store",
    redirect: "error",
    signal: AbortSignal.timeout(10000),
  });
  if (!response.ok) throw new Error("Unknown chat host");
  const body = await response.json();
  const id = body?.channel_id;
  if (typeof id !== "string" || !/^appchan_[0-9a-f]{32}$/.test(id))
    throw new Error("Unknown chat host");
  resolved.set(label, { id, at: Date.now() });
  return id;
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
export async function channelOf(request: Request) {
  return channelForHost(hostOf(request));
}
export function safeMutation(request: Request) {
  const origin = request.headers.get("origin");
  return origin === `https://${hostOf(request)}`;
}
const AGENT_LOGO = /^\/v1\/avatars\/(avatar_[0-9a-f]{32})\/circle-64\.png$/;

/** Same-origin path for this channel's agent avatar. Anything else is dropped. */
export function chatLogo(value: unknown): string | undefined {
  if (typeof value !== "string") return;
  const id = value.match(AGENT_LOGO)?.[1];
  return id ? `/api/chat/logo/${id}` : undefined;
}

export async function bootstrap(channel: string) {
  const response = await fetch(`${settings().upstream}/v1/channels/${channel}/chat/config`, {
    cache: "no-store",
    signal: AbortSignal.timeout(10000),
  });
  if (!response.ok) throw new Error("Chat unavailable");
  const config = await response.json();
  if (config && typeof config === "object") {
    config.logo = chatLogo(config.logo);
    // The management id stays on the server. The address bar already has the host.
    delete config.channel_id;
    delete config.public_url;
  }
  return config;
}
