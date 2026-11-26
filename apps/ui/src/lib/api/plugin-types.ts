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

// `PluginOrigin`, `PluginOption` and `WorkChatPluginList` come from the OpenAPI schema.
