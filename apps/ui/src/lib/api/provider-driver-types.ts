// Driver identity for model providers.
//
// Split out of legacy-api-types.ts rather than grown in place: that file is on
// the source-size ratchet's debt list and may not grow against the merge base,
// and `oxfmt` formats a union one member per line, so adding a driver there is
// a guaranteed guard failure. Re-exported through `./types`, so importers are
// unaffected.

/** Wire id of a model provider driver, matching the server's `DriverId`. */
export type DriverId =
  | "openai"
  | "chatgpt"
  | "openai-codex"
  | "openrouter"
  | "azure_openai"
  | "openai_completions"
  | "anthropic"
  | "gemini"
  | "bedrock"
  | "mai"
  | "fireworks"
  | "meta"
  | "cloudflare"
  | "vercel";
