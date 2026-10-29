// Synthetic local IdP and opaque BFF-session fixture. No real provider keys.
// Only public JWKS leave this process except through the coordinator's private
// credential manifest. Interactive login is outside the steady-state workload.
import { createServer } from "node:https";
import { generateKeyPairSync, sign, randomBytes, createHash, timingSafeEqual } from "node:crypto";
import { readFileSync, writeFileSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { seal } from "../../apps/chat-ui/src/server/crypto.ts";
const require = createRequire(new URL("../../apps/chat-ui/package.json", import.meta.url));
const { Pool } = require("pg");
const filename = process.argv[2];
if ((statSync(filename).mode & 0o077) !== 0)
  throw new Error("private fixture configuration required");
const cfg = JSON.parse(readFileSync(filename, "utf8"));
const pool = new Pool({ connectionString: cfg.chat_database, max: 5 });
const { privateKey, publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const jwk = {
  ...publicKey.export({ format: "jwk" }),
  kid: "capacity-local",
  alg: "RS256",
  use: "sig",
};
const jwks = { keys: [jwk] };
writeFileSync(cfg.public_file, JSON.stringify({ issuer: cfg.issuer, jwks }), { mode: 0o600 });
const refresh = new Map();
function mint(subject, variant = "valid") {
  const encode = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");
  const unsigned =
    encode({ alg: "RS256", kid: jwk.kid }) +
    "." +
    encode({
      iss: cfg.issuer,
      aud: variant === "audience" ? "wrong-audience" : cfg.client_id,
      sub: subject,
      iat: Math.floor(Date.now() / 1000),
      exp: Math.floor(Date.now() / 1000) + (variant === "expired" ? -300 : 900),
    });
  const signature = sign("RSA-SHA256", Buffer.from(unsigned), privateKey);
  if (variant === "signature") signature[0] ^= 1;
  return unsigned + "." + signature.toString("base64url");
}
function json(res, value, status = 200) {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(value));
}
function authorized(value) {
  const a = Buffer.from(value ?? ""),
    b = Buffer.from(cfg.client_secret);
  return a.length === b.length && timingSafeEqual(a, b);
}
await pool.query(
  "CREATE TABLE IF NOT EXISTS hosted_chat_credentials(token_hash TEXT PRIMARY KEY,kind TEXT NOT NULL,payload TEXT NOT NULL,expires_at TIMESTAMPTZ NOT NULL)",
);
const server = createServer(
  { cert: readFileSync(cfg.cert), key: readFileSync(cfg.cert_key) },
  async (req, res) => {
    try {
      if (req.url === "/.well-known/openid-configuration")
        return json(res, {
          issuer: cfg.issuer,
          authorization_endpoint: cfg.issuer + "/authorize",
          token_endpoint: cfg.issuer + "/token",
          jwks_uri: cfg.issuer + "/jwks",
        });
      if (req.url === "/jwks") return json(res, jwks);
      if (req.method !== "POST") return json(res, { error: "not_found" }, 404);
      let body = "";
      for await (const part of req) {
        body += part;
        if (body.length > 32768) return json(res, { error: "too_large" }, 413);
      }
      if (req.url === "/token") {
        const fields = new URLSearchParams(body);
        if (!authorized(fields.get("client_secret")) || fields.get("client_id") !== cfg.client_id)
          return json(res, { error: "unauthorized" }, 401);
        const subject = refresh.get(fields.get("refresh_token"));
        if (!subject) return json(res, { error: "invalid_grant" }, 400);
        refresh.delete(fields.get("refresh_token"));
        const next = randomBytes(32).toString("base64url");
        refresh.set(next, subject);
        return json(res, { id_token: mint(subject), refresh_token: next });
      }
      if (!authorized(req.headers["x-capacity-secret"]))
        return json(res, { error: "unauthorized" }, 401);
      if (req.url === "/verification-fixture") {
        const { variant } = JSON.parse(body);
        if (!["valid", "audience", "expired", "signature"].includes(variant))
          return json(res, { error: "invalid_fixture" }, 400);
        return json(res, { token: mint("capacity-verification", variant) });
      }
      if (req.url === "/provision") {
        const { subject, channel } = JSON.parse(body);
        if (typeof subject !== "string" || !/^appchan_[a-f0-9]{32}$/.test(channel))
          return json(res, { error: "invalid_fixture" }, 400);
        const opaque = randomBytes(32).toString("base64url"),
          hash = createHash("sha256").update(opaque).digest("hex");
        const rotation = randomBytes(32).toString("base64url");
        refresh.set(rotation, subject);
        const payload = {
          recoveryId: randomBytes(32).toString("base64url"),
          channel,
          identity: mint(subject),
          refresh: rotation,
          subject,
          expires: Date.now() / 1000 + 900,
        };
        await pool.query(
          "INSERT INTO hosted_chat_credentials(token_hash,kind,payload,expires_at) VALUES($1,'session',$2,NOW()+INTERVAL '4 hours')",
          [hash, seal(payload, cfg.session_key, "session:" + hash)],
        );
        return json(res, { cookie: "__Host-everruns_chat=" + opaque });
      }
      return json(res, { error: "not_found" }, 404);
    } catch {
      json(res, { error: "fixture_error" }, 500);
    }
  },
);
server.listen(cfg.idp_port, "127.0.0.1", () =>
  console.log("Local synthetic identity fixture ready"),
);
process.on("SIGTERM", () => {
  server.close();
  pool.end().finally(() => process.exit());
});
