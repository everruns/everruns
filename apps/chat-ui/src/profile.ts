export type Viewer = { name: string; email?: string; picture?: string };

const NAME_MAX = 120;
const EMAIL_MAX = 254;
const PICTURE_MAX = 2048;

function clean(value: unknown, max: number): string | undefined {
  if (typeof value !== "string") return;
  const text = value.trim();
  if (!text || text.length > max) return;
  if (/[\u0000-\u001f\u007f\u202a-\u202e\u2066-\u2069]/.test(text)) return;
  return text;
}

function emailOf(value: unknown, verified: unknown): string | undefined {
  if (verified === false) return;
  const email = clean(value, EMAIL_MAX);
  if (!email || !/^[^@\s]+@[^@\s]+$/.test(email)) return;
  return email;
}

function httpsPicture(value: unknown): string | undefined {
  const text = clean(value, PICTURE_MAX);
  if (!text) return;
  try {
    const url = new URL(text);
    if (url.protocol !== "https:" || url.username || url.password) return;
    return url.href;
  } catch {
    return;
  }
}

/** Display profile from verified identity claims. Drops anything that is not a name, email, or https picture. */
export function presentViewer(value: unknown): Viewer | null {
  if (!value || typeof value !== "object") return null;
  const claims = value as Record<string, unknown>;
  const email = emailOf(claims.email, claims.email_verified);
  const combined = [clean(claims.given_name, NAME_MAX), clean(claims.family_name, NAME_MAX)]
    .filter(Boolean)
    .join(" ");
  const name =
    clean(claims.name, NAME_MAX) ||
    clean(combined, NAME_MAX) ||
    clean(claims.preferred_username, NAME_MAX) ||
    email;
  if (!name) return null;
  const picture = httpsPicture(claims.picture);
  return {
    name,
    ...(email && email !== name ? { email } : {}),
    ...(picture ? { picture } : {}),
  };
}

function decodePayload(token: string): unknown {
  const part = token.split(".")[1];
  if (!part || token.split(".").length !== 3) return null;
  try {
    const b64 = part.replace(/-/g, "+").replace(/_/g, "/");
    const binary = atob(b64 + "=".repeat((4 - (b64.length % 4)) % 4));
    const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
    return JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    return null;
  }
}

/** Reads a sealed ID token payload. The caller has already verified the signature. */
export function viewerFromIdToken(token: string): Viewer | null {
  return presentViewer(decodePayload(token));
}
