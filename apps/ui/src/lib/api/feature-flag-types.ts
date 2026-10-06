export interface FeatureFlags {
  /** Integrated platform Chat workspace. Organization adoption opt-in. */
  chat_threads?: boolean;
  docker_capability?: boolean;
  container_sandbox?: boolean;
  lua?: boolean;
  openai_agents_api?: boolean;
  mcp_events?: boolean;
  /** Refuse agent-made changes without a reason. Organization adoption opt-in. */
  agent_change_reasons_required?: boolean;
  notifications: boolean;
  evals: boolean;
  /** Skills registry management UI. Experimental. */
  skills: boolean;
  /** Workspace memory management UI. Experimental. */
  memory: boolean;
  /** Knowledge index management UI. Experimental. */
  knowledge: boolean;
  /** Plugin marketplace and installed-plugin management UI. Experimental. */
  plugins: boolean;
  channel_budgets: boolean;
  agent_versions: boolean;
  voice: boolean;
  /** Outbound agent delegation (`a2a_agent_delegation`, `agent_handoff`). Experimental. */
  agent_delegation: boolean;
  /** Observers: online scoring of production sessions. Experimental. */
  observers: boolean;
  /** Public Chat (isolated public-facing chat web app + `public_chat` channel). Experimental. */
  public_chat: boolean;
  /** Browser-native tools exposed by the authenticated Everruns UI. Experimental. */
  webmcp: boolean;
  reports: boolean;
  /** Personal ChatGPT plan connections. Requires deployment and organization opt-in. */
  chatgpt_plan?: boolean;
  /** Machine-payment custody, policy, audit, and paid capability surfaces. */
  machine_payments: boolean;
}
