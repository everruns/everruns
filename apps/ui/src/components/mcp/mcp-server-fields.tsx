"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { McpElicitationPolicy } from "@/lib/api/types";
import type { McpServiceConnectionChoice } from "@/lib/form-validation";

/**
 * Elicitation policy select, shared by the create and edit dialogs.
 * Spec: knowledge/integrations/mcp-form-elicitation.md (D1).
 */
export function ElicitationPolicyField({
  id,
  value,
  onChange,
}: {
  id: string;
  value: McpElicitationPolicy;
  onChange: (value: McpElicitationPolicy) => void;
}) {
  const ui = useUiTranslations();
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{ui("Elicitation")}</Label>
      <Select value={value} onValueChange={(next) => onChange(next as McpElicitationPolicy)}>
        <SelectTrigger id={id}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="url">{ui("Links only")}</SelectItem>
          <SelectItem value="url_and_form">{ui("Links and questions")}</SelectItem>
          <SelectItem value="none">{ui("Off")}</SelectItem>
        </SelectContent>
      </Select>
      <p className="text-xs text-muted-foreground">
        {ui(
          "Whether this server may ask the user to open a link or answer questions mid-call. Questions are shown as coming from the server and never ask for passwords or keys.",
        )}
      </p>
    </div>
  );
}

/**
 * Where an agent's own (service) credential for this preset comes from: its
 * own MCP sign-in, or an existing agent connection such as its GitHub App.
 * Spec: knowledge/integrations/user-mcp-servers.md (step 5). The API only
 * accepts a provider on the hosts that take its tokens.
 */
export function ServiceConnectionField({
  id,
  value,
  onChange,
}: {
  id: string;
  value: McpServiceConnectionChoice;
  onChange: (value: McpServiceConnectionChoice) => void;
}) {
  const ui = useUiTranslations();
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{ui("Agent credential")}</Label>
      <Select value={value} onValueChange={(next) => onChange(next as McpServiceConnectionChoice)}>
        <SelectTrigger id={id}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="none">{ui("Sign in to this server")}</SelectItem>
          <SelectItem value="github">{ui("The agent's GitHub App")}</SelectItem>
        </SelectContent>
      </Select>
      <p className="text-xs text-muted-foreground">
        {ui(
          "What an agent acting as itself uses here. A connection is only accepted for servers on its provider's own hosts, such as the GitHub MCP server for GitHub.",
        )}
      </p>
    </div>
  );
}

/** Longest tool call timeout the API accepts, in seconds (30 minutes). */
export const MAX_TOOL_TIMEOUT_SECONDS = 1800;

/**
 * Parse the timeout field: `undefined` when empty (use the 60 s default),
 * milliseconds when valid, or an error message.
 */
export function parseToolTimeoutSeconds(
  value: string,
): { ms: number | undefined } | { error: string } {
  const trimmed = value.trim();
  if (trimmed === "") return { ms: undefined };
  const seconds = Number(trimmed);
  if (!Number.isInteger(seconds) || seconds < 1 || seconds > MAX_TOOL_TIMEOUT_SECONDS) {
    return { error: `Enter whole seconds from 1 to ${MAX_TOOL_TIMEOUT_SECONDS}` };
  }
  return { ms: seconds * 1000 };
}

/** Tool call timeout in seconds, shared by the create and edit dialogs. */
export function ToolTimeoutField({
  id,
  value,
  onChange,
  error,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
  error?: string;
}) {
  const ui = useUiTranslations();
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{ui("Tool call timeout (seconds)")}</Label>
      <Input
        id={id}
        type="number"
        inputMode="numeric"
        value={value}
        onChange={(e: React.ChangeEvent<HTMLInputElement>) => onChange(e.target.value)}
        aria-invalid={!!error}
        placeholder="60"
      />
      {error ? (
        <p className="text-xs text-destructive">{error}</p>
      ) : (
        <p className="text-xs text-muted-foreground">
          {ui(
            "How long one call to this server may run before it fails. Empty uses 60 seconds; the longest is 30 minutes.",
          )}
        </p>
      )}
    </div>
  );
}
