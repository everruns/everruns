// Real HTTPS browser and PostgreSQL broker. IdP/execution are explicit protocol
// fixtures; server domain tests qualify consumer execution authorization.
import assert from "node:assert/strict";
import { createServer as httpServer, request as proxyRequest } from "node:http";
import { createServer as httpsServer } from "node:https";
import { generateKeyPairSync, sign, randomUUID, createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { Pool } from "pg";
import { chromium } from "@playwright/test";
async function main() {
  const database = process.env.TEST_CHAT_DATABASE_URL;
  assert.ok(database, "TEST_CHAT_DATABASE_URL must name a disposable local database");
  const pool = new Pool({ connectionString: database }),
    schema = `chat_broker_${randomUUID().replaceAll("-", "")}`;
  const temporary = mkdtempSync(join(tmpdir(), "everruns-chat-broker-")),
    cert = join(temporary, "cert.pem"),
    key = join(temporary, "key.pem");
  execFileSync(
    "openssl",
    [
      "req",
      "-x509",
      "-newkey",
      "rsa:2048",
      "-nodes",
      "-keyout",
      key,
      "-out",
      cert,
      "-days",
      "1",
      "-subj",
      "/CN=localhost",
      "-addext",
      "subjectAltName=DNS:localhost,DNS:*.localhost",
    ],
    { stdio: "ignore" },
  );
  const url = new URL(database);
  url.searchParams.set("options", `-c search_path=${schema}`);
  const { privateKey, publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 }),
    jwk = { ...publicKey.export({ format: "jwk" }), kid: "fixture", alg: "RS256", use: "sig" };
  const codes = new Map<string, { nonce: string; challenge: string }>(),
    tls = { key: readFileSync(key), cert: readFileSync(cert) },
    issuer = "https://localhost:9151";
  const json = (response: import("node:http").ServerResponse, value: unknown) => {
    response.setHeader("Content-Type", "application/json");
    response.end(JSON.stringify(value));
  };
  const provider = httpsServer(tls, async (request, response) => {
    const u = new URL(request.url!, issuer);
    if (u.pathname === "/.well-known/openid-configuration")
      return json(response, {
        issuer,
        authorization_endpoint: `${issuer}/authorize`,
        token_endpoint: `${issuer}/token`,
        jwks_uri: `${issuer}/jwks`,
      });
    if (u.pathname === "/jwks") return json(response, { keys: [jwk] });
    if (u.pathname === "/authorize") {
      assert.equal(u.searchParams.get("redirect_uri"), "https://login.localhost/auth/callback");
      assert.equal(u.searchParams.get("code_challenge_method"), "S256");
      const code = randomUUID();
      codes.set(code, {
        nonce: u.searchParams.get("nonce")!,
        challenge: u.searchParams.get("code_challenge")!,
      });
      response.writeHead(303, {
        Location: `https://login.localhost/auth/callback?code=${code}&state=${u.searchParams.get("state")}`,
      });
      response.end();
      return;
    }
    if (u.pathname === "/token") {
      let body = "";
      for await (const part of request) body += part;
      const params = new URLSearchParams(body),
        code = params.get("code")!,
        pending = codes.get(code);
      codes.delete(code);
      if (
        !pending ||
        createHash("sha256").update(params.get("code_verifier")!).digest("base64url") !==
          pending.challenge
      ) {
        response.writeHead(400);
        response.end();
        return;
      }
      const header = Buffer.from(JSON.stringify({ alg: "RS256", kid: jwk.kid })).toString(
          "base64url",
        ),
        payload = Buffer.from(
          JSON.stringify({
            iss: issuer,
            aud: "broker-test",
            sub: "alice",
            nonce: pending.nonce,
            iat: Math.floor(Date.now() / 1000),
            exp: Math.floor(Date.now() / 1000) + 3600,
          }),
        ).toString("base64url");
      return json(response, {
        id_token: `${header}.${payload}.${sign("RSA-SHA256", Buffer.from(`${header}.${payload}`), privateKey).toString("base64url")}`,
        refresh_token: "provider-fixture-refresh",
      });
    }
    response.writeHead(404);
    response.end();
  });
  let admissions = 0;
  const api = httpServer((request, response) => {
    if (request.url?.endsWith("/chat/config"))
      return json(response, {
        name: "Broker integration",
        interface_mode: "multi_chat",
        features: { threads: false, files: false },
      });
    if (request.url?.endsWith("/runtime-auth")) {
      admissions++;
      return json(response, { access_token: "execution-fixture-token" });
    }
    if (request.url?.endsWith("/sessions")) return json(response, { data: [] });
    response.writeHead(404);
    response.end();
  });
  const edge = httpsServer(tls, (request, response) => {
    const upstream = proxyRequest(
      {
        hostname: "127.0.0.1",
        port: 9150,
        path: request.url,
        method: request.method,
        headers: {
          ...request.headers,
          "x-forwarded-host": request.headers.host,
          "x-forwarded-proto": "https",
          "x-forwarded-for": "127.0.0.1",
        },
      },
      (res) => {
        response.writeHead(res.statusCode!, res.headers);
        res.pipe(response);
      },
    );
    upstream.on("error", () => {
      response.writeHead(503);
      response.end();
    });
    request.pipe(upstream);
  });
  let next: ReturnType<typeof spawn> | undefined,
    browser: Awaited<ReturnType<typeof chromium.launch>> | undefined;
  try {
    await pool.query(`CREATE SCHEMA ${schema}`);
    await pool.query(
      `CREATE TABLE ${schema}.hosted_chat_credentials(token_hash TEXT PRIMARY KEY,kind TEXT NOT NULL,payload TEXT NOT NULL,expires_at TIMESTAMPTZ NOT NULL)`,
    );
    await Promise.all([
      new Promise<void>((r) => provider.listen(9151, "127.0.0.1", r)),
      new Promise<void>((r) => api.listen(9152, "127.0.0.1", r)),
      new Promise<void>((r) => edge.listen(443, r)),
    ]);
    next = spawn("node", ["node_modules/next/dist/bin/next", "dev", "--port", "9150"], {
      stdio: "inherit",
      env: {
        ...process.env,
        NODE_EXTRA_CA_CERTS: cert,
        CHAT_DOMAIN: "localhost",
        CHAT_LOGIN_LIMIT_PER_10_MINUTES: "6",
        CHAT_API_URL: "http://127.0.0.1:9152",
        CHAT_OIDC_ISSUER: issuer,
        CHAT_OIDC_CLIENT_ID: "broker-test",
        CHAT_OIDC_CLIENT_SECRET: "fixture-client",
        CHAT_SESSION_KEY: "12".repeat(32),
        CHAT_DATABASE_URL: url.toString(),
      },
    });
    const deadline = Date.now() + 60000;
    while (true) {
      try {
        if ((await fetch("http://127.0.0.1:9150/health")).ok) break;
      } catch {}
      assert.ok(Date.now() < deadline, "host readiness");
      await new Promise((r) => setTimeout(r, 250));
    }
    browser = await chromium.launch({ headless: true });
    const context = await browser.newContext({ ignoreHTTPSErrors: true }),
      page = await context.newPage(),
      host = "01933b5a000070008000000000000001.localhost",
      origin = `https://${host}`;

    let handoff = "";
    page.on("response", (res) => {
      const location = res.headers()["location"];
      if (location?.includes("/auth/complete?")) handoff = location;
    });
    await page.goto(`${origin}/auth/start?session=session_broker_test`);
    await page
      .waitForURL(`${origin}/?session=session_broker_test`, { timeout: 60000 })
      .catch(async (error) => {
        console.log("FAILED PAGE", await page.textContent("body"));
        throw error;
      });
    await page.waitForFunction(() => document.body.textContent?.includes("New conversation"));
    const cookies = await context.cookies(),
      session = cookies.find((cookie) => cookie.name === "__Host-everruns_chat");
    assert.ok(session);
    assert.equal(session.domain, host);
    assert.equal(session.secure, true);
    assert.equal(session.httpOnly, true);
    assert.equal(session.sameSite, "Lax");
    assert.ok(!cookies.some((cookie) => cookie.value.includes("provider-fixture")));
    const rows = await pool.query(`SELECT kind,payload FROM ${schema}.hosted_chat_credentials`);
    assert.equal(rows.rows.filter((row) => row.kind === "session").length, 1);
    assert.ok(rows.rows.every((row) => !row.payload.includes("provider-fixture")));
    assert.equal((await context.request.get(`${origin}/api/sessions`)).status(), 200);
    const recovery = await (await context.request.get(`${origin}/api/chat/recovery`)).json();
    assert.match(recovery.scope, /^[A-Za-z0-9_-]{43}$/);
    assert.notEqual(recovery.scope, session.value, "recovery namespace is not a credential");
    assert.deepEqual(
      await (await context.request.get(`${origin}/api/chat/recovery`)).json(),
      recovery,
    );

    assert.ok(handoff);
    assert.equal((await context.request.get(handoff)).status(), 400, "handoff replay");
    const sibling = "01933b5a000070008000000000000002.localhost";
    assert.equal(
      (
        await context.request.post(`${origin}/api/sessions`, {
          headers: { Origin: `https://${sibling}` },
          data: {},
        })
      ).status(),
      403,
      "sibling CSRF",
    );
    await context.addCookies([{ ...session, domain: sibling }]);
    assert.equal(
      (await context.request.get(`https://${sibling}/api/sessions`)).status(),
      401,
      "copied cookie is channel bound",
    );
    assert.equal(
      (
        await context.request.post(`${origin}/auth/logout`, { headers: { Origin: origin } })
      ).status(),
      204,
    );
    assert.equal(
      (await pool.query(`SELECT 1 FROM ${schema}.hosted_chat_credentials WHERE kind='session'`))
        .rowCount,
      0,
    );
    assert.equal((await context.request.get(`${origin}/api/sessions`)).status(), 401);
    assert.equal((await context.request.get(`${origin}/api/chat/recovery`)).status(), 401);
    assert.ok(admissions >= 2);
    let limited = 0;
    for (let attempt = 0; attempt < 6; attempt++) {
      limited = (await context.request.get(`${origin}/auth/start`, { maxRedirects: 0 })).status();
    }
    assert.equal(limited, 429, "pre-auth login admission is rate limited");
    console.log(
      "PASS: HTTPS OIDC/PKCE broker, durable one-use handoff, encrypted host session, sibling isolation, revocation",
    );
  } finally {
    await browser?.close();
    next?.kill("SIGTERM");
    for (const server of [edge, provider, api]) {
      server.closeAllConnections();
      server.close();
    }
    await pool.query(`DROP SCHEMA IF EXISTS ${schema} CASCADE`);
    await pool.end();
    rmSync(temporary, { recursive: true, force: true });
  }
}
main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
