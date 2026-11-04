// THREAT[TM-DOS-049]: bound pre-auth admission and opportunistic expiry cleanup.
// Opaque, revocable host sessions. Provider credentials are encrypted at rest;
// raw tokens never enter browser storage, URLs or client component props.
import { createHash, randomBytes } from "node:crypto";
import { seal, open } from "./crypto";
import { Pool } from "pg";
import { settings } from "./config";
let pool: Pool | undefined;
const db = () => (pool ??= new Pool({ connectionString: settings().database, max: 5 }));
export const random = () => randomBytes(32).toString("base64url");
export const hash = (value: string) => createHash("sha256").update(value).digest("hex");
export async function save(kind: string, value: unknown, seconds: number) {
  const token = random();
  await db().query(
    "INSERT INTO hosted_chat_credentials (token_hash,kind,payload,expires_at) VALUES ($1,$2,$3,NOW()+$4*INTERVAL '1 second')",
    [hash(token), kind, seal(value, settings().key, `${kind}:${hash(token)}`), seconds],
  );
  // Opportunistic bounded cleanup; never scan/delete all records in a request.
  await db().query(
    "DELETE FROM hosted_chat_credentials WHERE token_hash IN (SELECT token_hash FROM hosted_chat_credentials WHERE expires_at<NOW() LIMIT 50)",
  );
  return token;
}
export async function read<T>(kind: string, token: string, consume = false): Promise<T | null> {
  if (!/^[A-Za-z0-9_-]{43}$/.test(token)) return null;
  const sql = consume
    ? "DELETE FROM hosted_chat_credentials WHERE token_hash=$1 AND kind=$2 AND expires_at>NOW() RETURNING payload"
    : "SELECT payload FROM hosted_chat_credentials WHERE token_hash=$1 AND kind=$2 AND expires_at>NOW()";
  const result = await db().query(sql, [hash(token), kind]);
  return result.rows[0]
    ? open<T>(result.rows[0].payload, settings().key, `${kind}:${hash(token)}`)
    : null;
}
export async function remove(kind: string, token: string) {
  await db().query("DELETE FROM hosted_chat_credentials WHERE token_hash=$1 AND kind=$2", [
    hash(token),
    kind,
  ]);
}

// Refresh is serialized across replicas, so a rotating refresh token is spent
// once and a concurrent request reads its replacement after the row lock.
export async function updateSession<T>(
  token: string,
  operation: (value: T) => Promise<T | null>,
): Promise<T | null> {
  const connection = await db().connect();
  try {
    await connection.query("BEGIN");
    const result = await connection.query(
      "SELECT payload FROM hosted_chat_credentials WHERE token_hash=$1 AND kind='session' AND expires_at>NOW() FOR UPDATE",
      [hash(token)],
    );
    if (!result.rows[0]) {
      await connection.query("ROLLBACK");
      return null;
    }
    const value = await operation(
      open<T>(result.rows[0].payload, settings().key, `session:${hash(token)}`),
    );
    if (value)
      await connection.query("UPDATE hosted_chat_credentials SET payload=$2 WHERE token_hash=$1", [
        hash(token),
        seal(value, settings().key, `session:${hash(token)}`),
      ]);
    else
      await connection.query("DELETE FROM hosted_chat_credentials WHERE token_hash=$1", [
        hash(token),
      ]);
    await connection.query("COMMIT");
    return value;
  } catch (error) {
    await connection.query("ROLLBACK");
    throw error;
  } finally {
    connection.release();
  }
}
export async function ready() {
  await db().query("SELECT token_hash FROM hosted_chat_credentials LIMIT 1");
}

// The edge must overwrite X-Forwarded-For. Hash peer information with the
// deployment key so admission buckets contain no recoverable IP addresses.
export class LoginRateLimitError extends Error {}
export async function admitLogin(peer: string) {
  const limit = Number(process.env.CHAT_LOGIN_LIMIT_PER_10_MINUTES ?? 120);
  if (!Number.isSafeInteger(limit) || limit < 1 || limit > 10000)
    throw new Error("Invalid login admission limit");
  const source = hash(`login-limit:${settings().key}:${peer}`);
  const result = await db().query(
    `INSERT INTO hosted_chat_credentials (token_hash,kind,payload,expires_at)
    VALUES ($1,'limit','1',NOW()+INTERVAL '10 minutes')
    ON CONFLICT (token_hash) DO UPDATE SET
      payload=(CASE WHEN hosted_chat_credentials.expires_at>NOW() THEN hosted_chat_credentials.payload::int+1 ELSE 1 END)::text,
      expires_at=CASE WHEN hosted_chat_credentials.expires_at>NOW() THEN hosted_chat_credentials.expires_at ELSE NOW()+INTERVAL '10 minutes' END
    WHERE hosted_chat_credentials.kind='limit' RETURNING payload`,
    [source],
  );
  if (!result.rows[0] || Number(result.rows[0].payload) > limit)
    throw new LoginRateLimitError("Too many login attempts");
}
