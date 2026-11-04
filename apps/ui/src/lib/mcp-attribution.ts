/** Server-recorded OAuth client. Names are self-declared, never verified brands. */
export interface McpClientAttribution {
  client_id: string;
  grant_id: string;
  client_name: string;
}

export function mcpClientAttribution(value: unknown): McpClientAttribution | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  if (
    typeof record.client_id !== "string" ||
    typeof record.grant_id !== "string" ||
    typeof record.client_name !== "string"
  )
    return undefined;
  return {
    client_id: record.client_id,
    grant_id: record.grant_id,
    client_name: record.client_name,
  };
}
