"use client";

// Side sheet behind the agent page's "More" rows (and the Version history
// overflow item). Large editors (Branding, Starter files) need room a narrow
// column or an accordion cannot give, so each opens here, over the page.
//
// Two kinds of section live here and the sheet says which:
// - Draft sections (Branding, Starter files, Network access) edit the page's
//   draft. A change puts the page into edit mode; nothing saves until the
//   header's Save changes.
// - Live sections (MCP servers, Credentials, Version history) manage their own
//   resources and save as they go, as they did when they were tabs.

import { Check, Loader2, X, Zap } from "lucide-react";
import { useAgentNameAvailability } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { StartersEditor } from "@/components/starters-editor";
import { InitialFilesEditor } from "@/components/initial-files-editor";
import { NetworkAccessEditor } from "@/components/network-access-editor";
import { EnvironmentProfilesEditor } from "@/components/agents/environment-profiles-editor";
import { AgentMcpPanel } from "@/components/agents/agent-mcp-panel";
import { AgentCredentialsPanel } from "@/components/agents/agent-credentials-panel";
import { AgentHealthCheck } from "@/components/agents/agent-health-check";
import { AgentVersionHistory } from "@/components/agents/agent-version-history";
import type { AgentDraft } from "@/components/agents/use-agent-draft";
import type { Agent } from "@/lib/api/types";
import { formatTokens, pluralize } from "@/lib/formatting";
import { cn } from "@/lib/utils";

export type AgentSettingsSection =
  | "branding"
  | "mcp"
  | "credentials"
  | "files"
  | "network"
  | "environments"
  | "usage"
  | "health"
  | "versions";

const SECTIONS: Record<
  AgentSettingsSection,
  { title: string; description: string; kind: "draft" | "live" | "info"; wide?: boolean }
> = {
  branding: {
    title: "Branding",
    description: "How the agent is named and how it presents itself in chat.",
    kind: "draft",
  },
  mcp: {
    title: "MCP servers",
    description: "Servers whose tools this agent can call.",
    kind: "live",
    wide: true,
  },
  credentials: {
    title: "Credentials",
    description: "Secrets bound to tool parameters for this agent's runs.",
    kind: "live",
    wide: true,
  },
  files: {
    title: "Starter files",
    description: "Files copied into each new session for this agent.",
    kind: "draft",
    wide: true,
  },
  network: {
    title: "Network access",
    description:
      "Which hosts this agent's sessions can reach through network-capable tools. Narrows the harness policy; sessions can narrow it further.",
    kind: "draft",
  },
  environments: {
    title: "Environments",
    description: "Named execution profiles available when a new chat starts.",
    kind: "draft",
    wide: true,
  },
  usage: {
    title: "Token usage",
    description: "Tokens used across every session of this agent.",
    kind: "info",
  },
  health: {
    title: "Health check",
    description: "Generated smoke tests run against the agent's saved configuration.",
    kind: "info",
  },
  versions: {
    title: "Version history",
    description: "Published versions and snapshots of this agent.",
    kind: "live",
    wide: true,
  },
};

export function isAgentSettingsSection(value: string | null): value is AgentSettingsSection {
  return value !== null && value in SECTIONS;
}

interface AgentSettingsSheetProps {
  section: AgentSettingsSection | null;
  onOpenChange: (open: boolean) => void;
  agent: Agent;
  draft: AgentDraft;
  readOnly: boolean;
  /** Wraps a draft change so the page enters edit mode. */
  onDraftChange: <T>(apply: (value: T) => void) => (value: T) => void;
}

export function AgentSettingsSheet({
  section,
  onOpenChange,
  agent,
  draft,
  readOnly,
  onDraftChange,
}: AgentSettingsSheetProps) {
  const meta = section ? SECTIONS[section] : null;

  return (
    <Drawer open={section !== null} onOpenChange={onOpenChange}>
      <DrawerContent
        className={cn("gap-0 overflow-y-auto p-0", meta?.wide ? "sm:max-w-3xl" : "sm:max-w-xl")}
      >
        {section && meta && (
          <>
            <DrawerHeader className="border-b p-5 pr-12">
              <DrawerTitle>{meta.title}</DrawerTitle>
              <DrawerDescription>{meta.description}</DrawerDescription>
            </DrawerHeader>
            <div className="flex-1 p-5">
              {section === "branding" && (
                <BrandingSection
                  agent={agent}
                  draft={draft}
                  readOnly={readOnly}
                  onDraftChange={onDraftChange}
                />
              )}
              {section === "mcp" && <AgentMcpPanel agent={agent} />}
              {section === "credentials" && <AgentCredentialsPanel agentId={agent.id} />}
              {section === "files" && (
                <InitialFilesEditor
                  value={draft.files}
                  onChange={onDraftChange(draft.setFiles)}
                  disabled={readOnly}
                  description="Files copied into each new session for this agent."
                />
              )}
              {section === "network" && (
                <NetworkAccessEditor
                  value={draft.networkAccess}
                  onChange={onDraftChange(draft.setNetworkAccess)}
                  disabled={readOnly}
                  description="One pattern per line: example.com, *.example.com, or https://example.com/api/."
                />
              )}
              {section === "environments" && (
                <EnvironmentProfilesEditor
                  value={draft.environments}
                  onChange={onDraftChange(draft.setEnvironments)}
                  disabled={readOnly}
                />
              )}
              {section === "usage" && <UsageSection agent={agent} />}
              {section === "health" && <AgentHealthCheck agentId={agent.id} />}
              {section === "versions" && <AgentVersionHistory agent={agent} />}
            </div>
            <DrawerFooter className="items-center border-t p-4 sm:justify-between">
              <p className="text-xs text-muted-foreground">
                {meta.kind === "draft"
                  ? readOnly
                    ? "This agent is read-only."
                    : "Changes are kept with the page edit. Save changes applies them."
                  : meta.kind === "live"
                    ? "Changes here save immediately."
                    : ""}
              </p>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                Done
              </Button>
            </DrawerFooter>
          </>
        )}
      </DrawerContent>
    </Drawer>
  );
}

function BrandingSection({
  agent,
  draft,
  readOnly,
  onDraftChange,
}: {
  agent: Agent;
  draft: AgentDraft;
  readOnly: boolean;
  onDraftChange: AgentSettingsSheetProps["onDraftChange"];
}) {
  const { fields, errors } = draft;
  const nameAvailability = useAgentNameAvailability(draft.nameChanged ? fields.name : "", agent.id);
  const field = (key: Parameters<AgentDraft["setField"]>[0]) =>
    onDraftChange((value: string) => draft.setField(key, value));

  return (
    <div className="flex flex-col gap-5">
      <div className="space-y-2">
        <Label htmlFor="description">Description</Label>
        <Textarea
          id="description"
          placeholder="Describe what this agent does..."
          value={fields.description}
          onChange={(event) => field("description")(event.target.value)}
          disabled={readOnly}
          rows={2}
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="display_name">Display name</Label>
        <Input
          id="display_name"
          placeholder={fields.name ? undefined : "Customer Support Agent"}
          value={fields.display_name}
          onChange={(event) => field("display_name")(event.target.value)}
          disabled={readOnly}
        />
        <p className="text-xs text-muted-foreground">
          Shown in the UI. Defaults to the name when empty.
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="name">Name</Label>
        <Input
          id="name"
          placeholder="customer-support"
          value={fields.name}
          onChange={(event) => field("name")(event.target.value)}
          aria-invalid={!!errors.name}
          disabled={readOnly}
          className="font-mono"
          required
        />
        {errors.name && <p className="text-xs text-destructive">{errors.name}</p>}
        {draft.nameChanged && fields.name.length >= 2 && (
          <div className="flex items-center gap-1.5 text-xs">
            {nameAvailability.isChecking ? (
              <>
                <Loader2 className="size-3 animate-spin text-muted-foreground" />
                <span className="text-muted-foreground">Checking availability…</span>
              </>
            ) : nameAvailability.available === true ? (
              <>
                <Check className="size-3 text-success" />
                <span className="text-success">Name is available</span>
              </>
            ) : nameAvailability.available === false ? (
              <>
                <X className="size-3 text-destructive" />
                <span className="text-destructive">Name is already taken or invalid</span>
              </>
            ) : null}
          </div>
        )}
        <p className="text-xs text-muted-foreground">
          Unique identifier used in URLs and the API. Lowercase letters, numbers, and hyphens.
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="intro_markdown">Intro (Markdown)</Label>
        <Textarea
          id="intro_markdown"
          value={fields.intro_markdown}
          onChange={(event) => field("intro_markdown")(event.target.value)}
          placeholder={"Hey, I'm Ava. Ask me anything about your account."}
          disabled={readOnly}
          rows={4}
        />
        <p className="text-xs text-muted-foreground">
          Shown as an intro box on a fresh thread. Images are allowed. Hidden once the user types.
        </p>
        {errors.intro_markdown && (
          <p className="text-xs text-destructive">{errors.intro_markdown}</p>
        )}
      </div>

      <div className="space-y-2">
        <Label htmlFor="short_description">Short description</Label>
        <Input
          id="short_description"
          value={fields.short_description}
          onChange={(event) => field("short_description")(event.target.value)}
          placeholder="Answers account questions in seconds."
          disabled={readOnly}
          maxLength={2048}
        />
        <p className="text-xs text-muted-foreground">
          One line in simplified Markdown, shown below the chat title once the intro hides.
        </p>
        {errors.short_description && (
          <p className="text-xs text-destructive">{errors.short_description}</p>
        )}
      </div>

      {!readOnly && (
        <StartersEditor
          value={draft.starters}
          onChange={onDraftChange(draft.setStarters)}
          error={errors.starters}
        />
      )}
    </div>
  );
}

function UsageSection({ agent }: { agent: Agent }) {
  const usage = agent.usage;
  const sessions = agent.session_count ?? 0;
  const apps = agent.app_count ?? 0;

  return (
    <div className="flex flex-col gap-4 text-sm">
      {usage ? (
        <div className="flex items-center gap-3 border bg-muted/40 p-3">
          <Zap className="size-4 text-accent-foreground" />
          <div>
            <p className="font-medium">
              {formatTokens(usage.input_tokens + usage.output_tokens)} total
            </p>
            <p className="text-xs text-muted-foreground">
              {formatTokens(usage.input_tokens)} input / {formatTokens(usage.output_tokens)} output
              {usage.cache_read_tokens ? ` / ${formatTokens(usage.cache_read_tokens)} cached` : ""}
            </p>
          </div>
        </div>
      ) : (
        <p className="text-muted-foreground">No tokens used yet.</p>
      )}
      <p className="text-muted-foreground">
        Across {sessions} {pluralize(sessions, "session")} and {apps} {pluralize(apps, "app")}. The
        Stats tab breaks usage down over time.
      </p>
    </div>
  );
}
