// Model vendor identity.
//
// Split out of legacy-api-types.ts for the same reason as
// provider-driver-types.ts: that file is on the source-size ratchet's debt list
// and a new vendor is a new union member. Re-exported through `./types`.

/** Vendor/brand of a model, derived from the backend model registry. */
export type ModelVendor =
  | "openai"
  | "anthropic"
  | "google"
  | "nvidia"
  | "qwen"
  | "microsoft"
  | "meta"
  | "minimax"
  | "mistral"
  | "moonshot"
  | "typesafe"
  | "xai"
  | "llmsim";
