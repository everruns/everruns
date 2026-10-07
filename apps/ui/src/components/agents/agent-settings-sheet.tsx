"use client";

// Side sheet behind the agent page's "More" rows. Large editors (Branding, Files) need room a narrow
// column or an accordion cannot give, so each opens here, over the page.
//
// Two kinds of section live here and the sheet says which:
// - Draft sections (Branding, Files, Network access) edit the page's
//   draft. A change puts the page into edit mode; nothing saves until the
//   header's Save changes.
// - Live sections (MCP servers, Credentials, Service account) manage their own
//   resources and save as they go, as they did when they were tabs.

import { useState } from "react";
import { Check, Loader2, X, Zap } from "lucide-react";
import { useAgentNameAvailability } from "@/hooks";
import { AgentAvatarField } from "@/components/agents/agent-avatar-field";
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
import { SandboxPolicyEditor } from "@/components/agents/sandbox-policy-editor";
import { AgentMcpPanel } from "@/components/agents/agent-mcp-panel";
import { AgentCredentialsPanel } from "@/components/agents/agent-credentials-panel";
import { AgentHealthCheck } from "@/components/agents/agent-health-check";
import { AgentServiceAccount } from "@/components/agents/agent-service-account";
import { isReadOnlyStatus } from "@/lib/entity-lifecycle";
import type { AgentDraft } from "@/components/agents/use-agent-draft";
import type { Agent } from "@/lib/api/types";
import { formatTokens, pluralize } from "@/lib/formatting";
import { cn } from "@/lib/utils";

export type AgentSettingsSection =
  | "branding"
  | "mcp"
  | "credentials"
  | "service"
  | "files"
  | "network"
  | "sandbox"
  | "usage"
  | "health";

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
  service: {
    title: "Service account",
    description: "Connections the agent uses for operations configured to act as a service.",
    kind: "live",
  },
  files: {
    title: "Files",
    description:
      "Starting files for new sessions. Updating these files does not change existing sessions.",
    kind: "draft",
    wide: true,
  },
  network: {
    title: "Network access",
    description:
      "Which hosts this agent's sessions can reach through network-capable tools. Narrows the harness policy; sessions can narrow it further.",
    kind: "draft",
  },
  sandbox: {
    title: "Primary sandbox",
    description: "Sandbox bindings available when a new Playground Session starts.",
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
  fixedSandbox?: string;
  /** Wraps a draft change so the page enters edit mode. */
  onDraftChange: <T>(apply: (value: T) => void) => (value: T) => void;
}

export function AgentSettingsSheet({
  section,
  onOpenChange,
  agent,
  draft,
  readOnly,
  fixedSandbox,
  onDraftChange,
}: AgentSettingsSheetProps) {
  // Closing clears the parent's selection immediately; keep the editor and its width
  // until the drawer finishes exiting so it cannot flash an empty, narrower panel.
  const [contentSection, setContentSection] = useState(section);
  if (section !== null && section !== contentSection) {
    setContentSection(section);
  }
  const meta = contentSection ? SECTIONS[contentSection] : null;

  return (
    <Drawer
      open={section !== null}
      onOpenChange={onOpenChange}
      onOpenChangeComplete={(open) => {
        if (!open) setContentSection(null);
      }}
    >
      <DrawerContent
        className={cn("gap-0 overflow-y-auto p-0", meta?.wide ? "sm:max-w-3xl" : "sm:max-w-xl")}
      >
        {contentSection && meta && (
          <>
            <DrawerHeader className="border-b p-5 pr-12">
              <DrawerTitle>{meta.title}</DrawerTitle>
              <DrawerDescription>{meta.description}</DrawerDescription>
            </DrawerHeader>
            <div className="flex-1 p-5">
              {contentSection === "branding" && (
                <BrandingSection
                  agent={agent}
                  draft={draft}
                  readOnly={readOnly}
                  onDraftChange={onDraftChange}
                />
              )}
              {contentSection === "mcp" && <AgentMcpPanel agent={agent} />}
              {contentSection === "credentials" && <AgentCredentialsPanel agentId={agent.id} />}
              {contentSection === "service" && (
                <AgentServiceAccount
                  agentId={agent.id}
                  value={agent.service_virtual_user_id}
                  // Built-in agents reject definition edits, but this binding stays editable.
                  disabled={isReadOnlyStatus(agent.status)}
                />
              )}
              {contentSection === "files" && (
                <InitialFilesEditor
                  value={draft.files}
                  onChange={onDraftChange(draft.setFiles)}
                  disabled={readOnly}
                  description="Starting files for new sessions. Updating these files does not change existing sessions."
                />
              )}
              {contentSection === "network" && (
                <NetworkAccessEditor
                  value={draft.networkAccess}
                  onChange={onDraftChange(draft.setNetworkAccess)}
                  disabled={readOnly}
                  description="One pattern per line: example.com, *.example.com, or https://example.com/api/."
                />
              )}
              {contentSection === "sandbox" &&
                (fixedSandbox ? (
                  <div className="border bg-muted/40 p-4 text-sm">
                    <p className="font-medium">{fixedSandbox}</p>
                    <p className="text-muted-foreground">
                      Locked by the selected Harness. Agent and Session overrides are disabled.
                    </p>
                  </div>
                ) : (
                  <SandboxPolicyEditor
                    value={draft.sandboxPolicy}
                    onChange={onDraftChange(draft.setSandboxPolicy)}
                    disabled={readOnly}
                  />
                ))}
              {contentSection === "usage" && <UsageSection agent={agent} />}
              {contentSection === "health" && <AgentHealthCheck agentId={agent.id} />}
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
      <AgentAvatarField agent={agent} readOnly={readOnly} />

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
