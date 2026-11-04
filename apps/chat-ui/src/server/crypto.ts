// Bind ciphertext to its opaque credential hash. Even a writer of the session
// table cannot swap another consumer's encrypted provider tokens into a known
// cookie and impersonate that consumer.
import { createCipheriv, createDecipheriv, randomBytes } from "node:crypto";
function key(value: string) {
  if (!/^[a-f0-9]{64}$/i.test(value)) throw new Error("CHAT_SESSION_KEY must be 32 bytes in hex");
  return Buffer.from(value, "hex");
}
export function seal(value: unknown, keyHex: string, binding: string) {
  const iv = randomBytes(12);
  const cipher = createCipheriv("aes-256-gcm", key(keyHex), iv);
  cipher.setAAD(Buffer.from(binding));
  const bytes = Buffer.concat([cipher.update(JSON.stringify(value)), cipher.final()]);
  return Buffer.concat([iv, cipher.getAuthTag(), bytes]).toString("base64");
}
export function open<T>(value: string, keyHex: string, binding: string): T {
  const bytes = Buffer.from(value, "base64");
  const cipher = createDecipheriv("aes-256-gcm", key(keyHex), bytes.subarray(0, 12));
  cipher.setAAD(Buffer.from(binding));
  cipher.setAuthTag(bytes.subarray(12, 28));
  return JSON.parse(Buffer.concat([cipher.update(bytes.subarray(28)), cipher.final()]).toString());
}
