"use client";

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
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>Elicitation</Label>
      <Select value={value} onValueChange={(next) => onChange(next as McpElicitationPolicy)}>
        <SelectTrigger id={id}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="url">Links only</SelectItem>
          <SelectItem value="url_and_form">Links and questions</SelectItem>
          <SelectItem value="none">Off</SelectItem>
        </SelectContent>
      </Select>
      <p className="text-xs text-muted-foreground">
        Whether this server may ask the user to open a link or answer questions mid-call. Questions
        are shown as coming from the server and never ask for passwords or keys.
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
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>Agent credential</Label>
      <Select value={value} onValueChange={(next) => onChange(next as McpServiceConnectionChoice)}>
        <SelectTrigger id={id}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="none">Sign in to this server</SelectItem>
          <SelectItem value="github">The agent&apos;s GitHub App</SelectItem>
        </SelectContent>
      </Select>
      <p className="text-xs text-muted-foreground">
        What an agent acting as itself uses here. A connection is only accepted for servers on its
        provider&apos;s own hosts, such as the GitHub MCP server for GitHub.
      </p>
    </div>
  );
}
