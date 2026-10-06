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
  | "typesafe"
  | "azure_openai"
  | "openai_completions"
  | "anthropic"
  | "gemini"
  | "bedrock"
  | "mai"
  | "fireworks"
  | "meta"
  | "mistral"
  | "cloudflare"
  | "vercel";

/**
 * Drivers the server lists in `/v1/providers/config` only when an org flag
 * allows them (`chatgpt_plan`, `mistral_provider`). The server's list is the
 * source of truth, so a gated driver is offered only once the config names it.
 */
const SERVER_GATED_DRIVERS: readonly DriverId[] = ["chatgpt", "mistral"];

export function isDriverOffered(
  driver: DriverId,
  configDrivers: readonly { driver: string }[] | undefined,
): boolean {
  return (
    !SERVER_GATED_DRIVERS.includes(driver) ||
    (configDrivers?.some((entry) => entry.driver === driver) ?? false)
  );
}
