const TOKEN_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

export const DEFAULT_CHANNEL_TOKEN_LENGTH = 63;

export function generateChannelToken(length = DEFAULT_CHANNEL_TOKEN_LENGTH): string {
  if (length <= 0) return "";

  const cryptoApi = globalThis.crypto;
  if (!cryptoApi?.getRandomValues) {
    throw new Error("Secure random token generation is unavailable");
  }

  const bytes = new Uint8Array(length);
  cryptoApi.getRandomValues(bytes);
  return Array.from(bytes, (byte) => TOKEN_ALPHABET[byte % TOKEN_ALPHABET.length]).join("");
}
