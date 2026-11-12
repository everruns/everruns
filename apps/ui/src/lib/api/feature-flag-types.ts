export interface FeatureFlags {
  /** Worker memory consolidation. Organization adoption opt-in. */
  dreaming?: boolean;
  /** Organization execution monitoring. Adoption opt-in. */
  monitoring?: boolean;
  docker_capability?: boolean;
  container_sandbox?: boolean;
  lua?: boolean;
  /** Behavioral evals. Organization adoption opt-in. */
  evals: boolean;
  /** Skills registry management UI. Experimental. */
  skills: boolean;
  /** Workspace memory management UI. Experimental. */
  memory: boolean;
  /** Unified Knowledge files and optional search. Organization adoption opt-in. */
  knowledge: boolean;
  /** Plugin marketplace and installed-plugin management UI. Experimental. */
  plugins: boolean;
  voice: boolean;
  /** Observers: online scoring of production sessions. Organization adoption opt-in. */
  observers: boolean;
  /** Public Chat (isolated public-facing chat web app + `public_chat` channel). Experimental. */
  public_chat: boolean;
  reports: boolean;
  /** First-party Mistral AI provider. Off unless the deployment raises it. */
  mistral?: boolean;
  /** Machine-payment custody, policy, audit, and paid capability surfaces. */
  machine_payments: boolean;
  /** Runs the org's sessions on the Everruns Engine. Platform rollout switch. */
  engine_hosting?: boolean;
  /**
   * The org's sessions run on a data plane (v2). Derived by the server from
   * the org's data plane, never set by hand. Stage 1 serves chat only, so
   * session files, MCP and sandboxes are hidden.
   */
  data_plane?: boolean;
}
