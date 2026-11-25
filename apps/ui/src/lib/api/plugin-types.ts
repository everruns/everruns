// Plugin environment and personal plugin lists. Augments legacy-api-types.ts rather than
// growing it: that file is on the source-size ratchet.

declare module "./legacy-api-types" {
  interface InstalledPlugin {
    /** What the plugin needs from the agent that runs it */
    environment?: PluginEnvironment;
    /** Whether the plugin declared it (`extensions["com.everruns"].environment`) */
    environment_declared?: boolean;
  }
}

/** What a plugin needs from the agent, weakest first. */
export type PluginEnvironment = "conversation" | "workspace" | "virtual-shell" | "sandbox";

/** Where a plugin in a person's list comes from. */
export type PluginOrigin = "agent" | "added" | "available";

/** One approved plugin as a person sees it (`/users/me/plugins`, Work Chat). */
export interface PluginOption {
  id: string;
  name: string;
  display_name: string | null;
  description: string;
  version: string | null;
  environment: PluginEnvironment;
  origin: PluginOrigin;
  /** In this person's new conversations */
  enabled: boolean;
  /** The agent provides what the plugin needs */
  fits: boolean;
  /** An admin still has to choose how one of its MCP servers signs in */
  needs_admin_setup: boolean;
}

/** A person's plugins in one Work Chat channel. */
export interface WorkChatPluginList {
  allow_adding: boolean;
  allow_turning_off: boolean;
  plugins: PluginOption[];
}
