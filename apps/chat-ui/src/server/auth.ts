// THREAT[TM-AUTH-039]: browser-bound, one-use handoffs and host-only opaque sessions.
import { createHash, createPublicKey, verify } from "node:crypto";
import { cookies } from "next/headers";
import { settings, channelOf, hostOf, bootstrap } from "./config";
import { random, hash, save, read, remove, updateSession, admitLogin } from "./store";
const SESSION = "__Host-everruns_chat",
  BROWSER = "__Host-everruns_chat_flow",
  BROKER = "__Host-everruns_chat_broker";
const attrs = { httpOnly: true, secure: true, sameSite: "lax" as const, path: "/" };
type Flow = {
  channel: string;
  browser: string;
  verifier: string;
  nonce: string;
  selectedSession?: string;
};
type Session = {
  channel: string;
  identity: string;
  refresh?: string;
  subject: string;
  expires: number;
};
async function discovery() {
  const cfg = settings();
  const response = await fetch(`${cfg.issuer}/.well-known/openid-configuration`, {
    signal: AbortSignal.timeout(10000),
    cache: "no-store",
    redirect: "error",
  });
  if (!response.ok) throw new Error("Identity unavailable");
  const metadata = await response.json();
  if (metadata.issuer !== cfg.issuer) throw new Error("Unexpected issuer");
  for (const name of ["authorization_endpoint", "token_endpoint", "jwks_uri"]) {
    if (new URL(metadata[name]).origin !== new URL(cfg.issuer).origin)
      throw new Error("Unexpected identity endpoint");
  }
  return metadata;
}
async function identity(token: string, nonce?: string) {
  const cfg = settings(),
    parts = token.split(".");
  if (parts.length !== 3 || token.length > 16384) throw new Error("Invalid identity");
  const header = JSON.parse(Buffer.from(parts[0], "base64url").toString());
  if (header.alg !== "RS256") throw new Error("Invalid algorithm");
  const metadata = await discovery();
  const response = await fetch(metadata.jwks_uri, {
    signal: AbortSignal.timeout(10000),
    redirect: "error",
  });
  if (!response.ok) throw new Error("Identity unavailable");
  const keys = await response.json();
  const jwk = keys.keys.find(
    (key: { kid: string; kty: string }) => key.kid === header.kid && key.kty === "RSA",
  );
  if (
    !jwk ||
    !verify(
      "RSA-SHA256",
      Buffer.from(`${parts[0]}.${parts[1]}`),
      createPublicKey({ key: jwk, format: "jwk" }),
      Buffer.from(parts[2], "base64url"),
    )
  )
    throw new Error("Invalid signature");
  const claims = JSON.parse(Buffer.from(parts[1], "base64url").toString());
  const now = Date.now() / 1000;
  if (
    claims.iss !== cfg.issuer ||
    claims.aud !== cfg.clientId ||
    typeof claims.sub !== "string" ||
    !claims.sub ||
    !Number.isFinite(claims.exp) ||
    claims.exp <= now ||
    claims.iat > now + 60 ||
    (nonce !== undefined && claims.nonce !== nonce)
  )
    throw new Error("Invalid claims");
  return claims as { sub: string; exp: number };
}
export async function tokenFor(request: Request): Promise<string | null> {
  const channel = channelOf(request),
    jar = await cookies(),
    token = jar.get(SESSION)?.value;
  if (!token) return null;
  const session = await read<Session>("session", token);
  if (!session || session.channel !== channel) return null;
  let current = session;
  if (current.expires < Date.now() / 1000 + 60) {
    const refreshed = await updateSession<Session>(token, async (value) => {
      if (value.channel !== channel) return null;
      if (value.expires >= Date.now() / 1000 + 60) return value;
      if (!value.refresh) return null;
      const cfg = settings(),
        metadata = await discovery();
      const response = await fetch(metadata.token_endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({
          grant_type: "refresh_token",
          refresh_token: value.refresh,
          client_id: cfg.clientId,
          client_secret: cfg.clientSecret,
        }),
        signal: AbortSignal.timeout(10000),
        redirect: "error",
      });
      if (!response.ok) return null;
      const tokens = await response.json(),
        claims = await identity(tokens.id_token);
      if (claims.sub !== value.subject) return null;
      return {
        ...value,
        identity: tokens.id_token,
        refresh: tokens.refresh_token ?? value.refresh,
        expires: claims.exp,
      };
    });
    if (!refreshed) return null;
    current = refreshed;
  }
  // Exchange proves the provider binding and rechecks channel liveness/policy
  // on every request. A browser session never renews runtime authority itself.
  const response = await fetch(`${settings().upstream}/v1/channels/${channel}/runtime-auth`, {
    method: "POST",
    headers: { Authorization: `Bearer ${current.identity}` },
    cache: "no-store",
    signal: AbortSignal.timeout(10000),
  });
  if (!response.ok) return null;
  return (await response.json()).access_token;
}
export async function authenticate(request: Request, action: string): Promise<Response> {
  const cfg = settings(),
    jar = await cookies(),
    url = new URL(request.url);
  if (action === "start") {
    await admitLogin(request.headers.get("x-forwarded-for") ?? "direct");
    const channel = channelOf(request);
    await bootstrap(channel);
    const browser = random();
    const selected = url.searchParams.get("session");
    const selectedSession =
      selected && selected.length <= 128 && /^session_[A-Za-z0-9_-]+$/.test(selected)
        ? selected
        : undefined;
    jar.set(BROWSER, browser, { ...attrs, maxAge: 600 });
    const flow = await save(
      "flow",
      {
        channel,
        browser: hash(browser),
        verifier: random(),
        nonce: random(),
        selectedSession,
      } satisfies Flow,
      600,
    );
    return Response.redirect(`${cfg.broker}/auth/authorize?flow=${encodeURIComponent(flow)}`, 303);
  }
  if (hostOf(request) === `login.${cfg.domain}`) {
    if (action === "authorize") {
      await admitLogin(request.headers.get("x-forwarded-for") ?? "direct");
      const flowId = url.searchParams.get("flow") ?? "",
        flow = await read<Flow>("flow", flowId);
      if (!flow) throw new Error("Expired login");
      jar.set(BROKER, flowId, { ...attrs, maxAge: 600 });
      const metadata = await discovery();
      const target = new URL(metadata.authorization_endpoint);
      target.search = new URLSearchParams({
        client_id: cfg.clientId,
        redirect_uri: `${cfg.broker}/auth/callback`,
        response_type: "code",
        scope: "openid profile email offline_access",
        state: flowId,
        nonce: flow.nonce,
        code_challenge: createHash("sha256").update(flow.verifier).digest("base64url"),
        code_challenge_method: "S256",
      }).toString();
      return Response.redirect(target, 303);
    }
    if (action === "callback") {
      const state = url.searchParams.get("state") ?? "";
      if (!state || jar.get(BROKER)?.value !== state) throw new Error("Invalid login state");
      const flow = await read<Flow>("flow", state, true);
      if (!flow) throw new Error("Expired login");
      jar.set(BROKER, "", { ...attrs, maxAge: 0 });
      const metadata = await discovery();
      const response = await fetch(metadata.token_endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({
          grant_type: "authorization_code",
          code: url.searchParams.get("code") ?? "",
          redirect_uri: `${cfg.broker}/auth/callback`,
          client_id: cfg.clientId,
          client_secret: cfg.clientSecret,
          code_verifier: flow.verifier,
        }),
        redirect: "error",
        signal: AbortSignal.timeout(10000),
      });
      if (!response.ok) throw new Error("Sign-in failed");
      const tokens = await response.json(),
        claims = await identity(tokens.id_token, flow.nonce);
      const code = await save(
        "handoff",
        {
          ...flow,
          identity: tokens.id_token,
          refresh: tokens.refresh_token,
          subject: claims.sub,
          expires: claims.exp,
        },
        60,
      );
      const label = flow.channel.slice("appchan_".length);
      return Response.redirect(
        `https://${label}.${cfg.domain}/auth/complete?code=${encodeURIComponent(code)}`,
        303,
      );
    }
    throw new Error("Unknown login route");
  }
  const channel = channelOf(request);
  if (action === "complete") {
    const code = url.searchParams.get("code") ?? "",
      pending = await read<Flow & Session>("handoff", code);
    const browser = jar.get(BROWSER)?.value;
    if (!pending || pending.channel !== channel || !browser || hash(browser) !== pending.browser)
      throw new Error("Invalid login destination");
    const consumed = await read<Flow & Session>("handoff", code, true);
    if (!consumed) throw new Error("Login already used");
    await bootstrap(channel);
    const admitted = await fetch(`${cfg.upstream}/v1/channels/${channel}/runtime-auth`, {
      method: "POST",
      headers: { Authorization: `Bearer ${consumed.identity}` },
      signal: AbortSignal.timeout(10000),
      cache: "no-store",
    });
    if (!admitted.ok) throw new Error("This identity cannot access this chat");
    const session = await save(
      "session",
      {
        channel,
        identity: consumed.identity,
        refresh: consumed.refresh,
        subject: consumed.subject,
        expires: consumed.expires,
      } satisfies Session,
      30 * 86400,
    );
    jar.set(SESSION, session, { ...attrs, maxAge: 30 * 86400 });
    jar.set(BROWSER, "", { ...attrs, maxAge: 0 });
    const destination = consumed.selectedSession
      ? `?session=${encodeURIComponent(consumed.selectedSession)}`
      : "";
    return Response.redirect(`https://${hostOf(request)}/${destination}`, 303);
  }
  throw new Error("Unknown login route");
}
export async function logout() {
  const jar = await cookies(),
    token = jar.get(SESSION)?.value;
  if (token) await remove("session", token);
  jar.set(SESSION, "", { ...attrs, maxAge: 0 });
}
