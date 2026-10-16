export const CODING_AGENT_SETUP_URL = "https://everruns.com/agent-setup/prompt.md";
export const CODING_AGENT_MANUAL_URL = "https://everruns.com/docs/getting-started/use-in-ai-tools/";
export const CONNECTED_AI_CLIENTS_HREF = "/settings/agent-experience#connected-ai-clients";

// Carry the deployment explicitly: the distributed plugin defaults to Cloud,
// while this dialog also ships in self-hosted and local installations.
export function codingAgentSetupPrompt(mcpUrl: string, requiresAuth: boolean): string {
  return [
    `Set me up for Everruns using the instructions at ${CODING_AGENT_SETUP_URL}`,
    `Use this Everruns MCP endpoint: ${mcpUrl}`,
    requiresAuth
      ? "Use the client's browser OAuth sign-in when required."
      : "This Everruns deployment does not require authentication.",
  ].join("\n\n");
}
