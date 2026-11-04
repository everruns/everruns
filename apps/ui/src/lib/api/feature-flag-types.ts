export interface FeatureFlags {
  /** Worker memory consolidation. Organization adoption opt-in. */
  dreaming?: boolean;
  /** Page-aware split Chat. Organization Adoption opt-in. */
  chat_mode?: boolean;
  /** Organization execution monitoring. Adoption opt-in. */
  monitoring?: boolean;
  /** Chat delegation and its background Work panel. Organization Adoption opt-in. */
  chat_threads?: boolean;
  docker_capability?: boolean;
  container_sandbox?: boolean;
  lua?: boolean;
  openai_agents_api?: boolean;
  mcp_events?: boolean;
  /** Refuse agent-made changes without a reason. Organization adoption opt-in. */
  agent_change_reasons_required?: boolean;
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
  /** Observers: online scoring of production sessions. Experimental. */
  observers: boolean;
  /** Public Chat (isolated public-facing chat web app + `public_chat` channel). Experimental. */
  public_chat: boolean;
  reports: boolean;
  /** First-party Mistral AI provider. Off unless the deployment raises it. */
  mistral?: boolean;
  /** Machine-payment custody, policy, audit, and paid capability surfaces. */
  machine_payments: boolean;
}
