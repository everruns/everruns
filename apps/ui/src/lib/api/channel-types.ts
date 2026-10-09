// The voice channel config. Split out of legacy-api-types.ts,
// which is over the file-size limit; re-exported from there.

// `ChannelType` is generated from the OpenAPI schema (schema-types.ts).

export type VoiceTurnDetection = "server_vad" | "semantic_vad";
export type VoiceInterruption = "steer" | "cancel";

/**
 * Voice channel configuration. Mirrors `VoiceChannelConfig` in
 * `crates/contracts/src/voice.rs`. Voice channels take no `auth` block.
 */
export interface VoiceChannelConfig {
  /** Listening/thinking split; only `delegated` (the agent writes every answer). */
  mode?: "delegated";
  /** Speech model id, e.g. `gpt-realtime-2`. */
  model?: string;
  /** Provider voice, e.g. `marin`. */
  voice?: string;
  /** Optional BCP 47 language hint, e.g. `en`. */
  language?: string;
  /** Spoken when the call connects (max 500 chars). */
  greeting?: string;
  turn_detection?: VoiceTurnDetection;
  interruption?: VoiceInterruption;
  /** Silence before the filler line, in ms; 0 disables fillers (max 60000). */
  filler_after_ms?: number;
  filler?: string;
  /** Extra speaking-style instructions for the speech model (max 4000 chars). */
  speaking_style?: string;
}
