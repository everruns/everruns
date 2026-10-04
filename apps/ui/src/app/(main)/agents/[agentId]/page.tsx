"use client";

import { exportAgentPackage } from "@/lib/api/agents";

// Agent page: one layout for reading and editing an agent.
//
// The system prompt is the page (wide left pane); a narrow config column holds
// the primary settings and a quiet "More" list whose rows open side sheets.
// Edit mode is page-level and in place: the same panes turn writable, the
// header swaps Test in Playground for Save changes / Discard, and one Save sends the
// whole draft so a prompt edit and the capability change that goes with it
// land together. The old /edit route redirects here with `?mode=edit`.
//
// One tab row only: Agent · Preview · Integrations · Stats · Sessions. MCP and
// Credentials are configuration (config column sheets); Versions and the old
// danger zone live in the header overflow menu.

import { use, useCallback, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import {
  Archive,
  Boxes,
  Check,
  Copy,
  Download,
  GitBranch,
  MessageCircle,
  MoreHorizontal,
  Pencil,
  Telescope,
  Trash2,
} from "lucide-react";
import {
  useAgent,
  useAgentCredentials,
  useAgentMcpAttachments,
  useAgentStats,
  useCapabilities,
  useCopyAgent,
  useDeleteAgent,
  useDestroyAgent,
  useExportAgent,
  useLatestHealthCheckRun,
  useModels,
  usePageTitle,
  useUpdateAgent,
} from "@/hooks";
import { usePolicies } from "@/hooks/use-policies";
import { useWebMcpTool } from "@/hooks/use-webmcp-tool";
import { ResourceNotFound } from "@/components/resource-not-found";
import { EntityDeleteErrorNotice } from "@/components/entity-delete-error-notice";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  BackLink,
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageFooter,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";
import { AgentChecks, applyByteSpanReplacement } from "@/components/agents/agent-checks";
import { AgentServiceAccount } from "@/components/agents/agent-service-account";
import { AgentConfigColumn, type AgentMoreRow } from "@/components/agents/agent-config-column";
import { AgentIntegrationsPanel } from "@/components/agents/agent-integrations-panel";
import { AgentPreview } from "@/components/agents/agent-preview";
import { AgentPromptPane } from "@/components/agents/agent-prompt-pane";
import { AgentSessionsPanel } from "@/components/agents/agent-sessions-panel";
import {
  AgentSettingsSheet,
  type AgentSettingsSection,
} from "@/components/agents/agent-settings-sheet";
import {
  agentTabHref,
  getAgentTabItems,
  isAgentTab,
  resolveAgentTab,
  type AgentTab,
} from "@/components/agents/agent-tabs";
import { BRANDING_FIELDS, useAgentDraft } from "@/components/agents/use-agent-draft";
import { ResourceStatsPanel } from "@/components/stats/resource-stats-panel";
import { normalizeNetworkAccess } from "@/components/network-access-editor";
import type { LatestHealthCheckRun, ModelWithProvider } from "@/lib/api/types";
import type { WebMcpToolDefinition } from "@/lib/webmcp/types";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { formatTokens, pluralize } from "@/lib/formatting";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { useWebMcp } from "@/providers/webmcp-context";

const count = (n: number, singular: string, plural?: string) =>
  `${n} ${pluralize(n, singular, plural)}`;

function healthSummary(latest: LatestHealthCheckRun | undefined): string {
  const run = latest?.run;
  if (!run) return "Not run";
  if (run.status === "pending" || run.status === "running") return "Running";
  if (run.status === "failed") return "Failed";
  const result = run.summary ? `${run.summary.passed}/${run.summary.total} passed` : "Completed";
  return latest.config_changed ? `${result} · outdated` : result;
}

export default function AgentDetailPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { agentId } = use(params);
  const router = useRouter();
  const searchParams = useSearchParams();
  const tabParam = searchParams.get("tab");
  const deepLink = resolveAgentTab(tabParam);
  const [activeTab, setActiveTab] = useState<AgentTab>(deepLink.tab);
  const [openSection, setOpenSection] = useState<AgentSettingsSection | null>(deepLink.section);
  // Follow the address bar when it changes without a click (refresh already
  // lands here; back/forward and a replaced query must too).
  const [tabParamSeen, setTabParamSeen] = useState<string | null>(tabParam);
  if (tabParam !== tabParamSeen) {
    setTabParamSeen(tabParam);
    setActiveTab(deepLink.tab);
  }
  const [editRequested, setEditRequested] = useState(() => searchParams.get("mode") === "edit");
  const [confirmAction, setConfirmAction] = useState<"archive" | "delete" | null>(null);

  const agentVersionsEnabled = useFeatureFlag("agent_versions");
  const observersEnabled = useFeatureFlag("observers");
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  usePageTitle(agent ? getDisplayName(agent) : null, "Agent");
  const { data: allCapabilities } = useCapabilities({ includeRetired: true });
  const { data: models } = useModels();
  const { data: stats, isLoading: statsLoading, error: statsError } = useAgentStats(agentId);
  const { data: mcpAttachments } = useAgentMcpAttachments(agentId);
  const { data: credentials } = useAgentCredentials(agentId);
  const { data: latestHealth } = useLatestHealthCheckRun(agentId);
  const updateAgent = useUpdateAgent();
  const deleteAgent = useDeleteAgent();
  const destroyAgent = useDestroyAgent();
  const exportAgent = useExportAgent();
  const [exporting, setExporting] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);
  const copyAgent = useCopyAgent();
  const { can } = usePolicies("agents");
  const webmcp = useWebMcp();

  const draft = useAgentDraft(agent);
  const readOnly = isReadOnlyStatus(agent?.status);
  const editing = editRequested && !!agent && !readOnly;

  const modelMap = useMemo(
    () => new Map<string, ModelWithProvider>((models ?? []).map((m) => [m.id, m])),
    [models],
  );
  const defaultModel = agent?.default_model_id ? modelMap.get(agent.default_model_id) : undefined;

  // Leaving the page with unsaved edits asks first.
  useEffect(() => {
    if (!editing || !draft.isDirty) return;
    const warn = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [editing, draft.isDirty]);

  const startEdit = useCallback(() => setEditRequested(true), []);
  const onDraftChange = useCallback(
    <T,>(apply: (value: T) => void) =>
      (value: T) => {
        apply(value);
        setEditRequested(true);
      },
    [],
  );

  const selectTab = useCallback(
    (tab: AgentTab) => {
      setActiveTab(tab);
      const href = agentTabHref(agentId, tab, searchParams);
      const query = searchParams.toString();
      const currentHref = query ? `/agents/${agentId}?${query}` : `/agents/${agentId}`;
      if (href !== currentHref) router.replace(href, { scroll: false });
    },
    [agentId, router, searchParams],
  );

  const exitEdit = () => {
    setEditRequested(false);
    if (searchParams.get("mode") !== "edit") return;
    const params = new URLSearchParams(searchParams.toString());
    params.delete("mode");
    router.replace(agentTabHref(agentId, activeTab, params), { scroll: false });
  };

  const handleDiscard = () => {
    draft.reset();
    updateAgent.reset();
    exitEdit();
  };

  const handleSave = async () => {
    const result = draft.buildRequest();
    if (!result.ok) {
      selectTab("agent");
      if (BRANDING_FIELDS.some((field) => result.errors[field])) setOpenSection("branding");
      return;
    }
    try {
      await updateAgent.mutateAsync({ agentId, request: result.request });
      draft.reset();
      exitEdit();
    } catch (error) {
      console.error("Failed to update agent:", error);
    }
  };

  const playgroundPath = `/playground/new?agent=${encodeURIComponent(agentId)}`;
  const handleTestAgent = () => router.push(playgroundPath);

  const openPlaygroundTool = useMemo<WebMcpToolDefinition>(
    () => ({
      name: "everruns_open_playground",
      description: "Open Playground with the agent displayed on this Everruns page selected.",
      inputSchema: {
        type: "object",
        properties: {},
        additionalProperties: false,
      },
      annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true },
      execute: async () => {
        webmcp.assertBinding(webmcp.bindingToken);
        if (!agent || agent.id !== agentId || agent.status !== "active") {
          throw new DOMException("The bound agent is no longer active", "AbortError");
        }
        await webmcp.requestApproval({
          title: "Open this agent in Playground?",
          description: `Open Playground with ${getDisplayName(agent)} selected. No session, sandbox, or model usage starts until you confirm the Playground setup.`,
          confirmLabel: "Open Playground",
        });
        webmcp.assertBinding(webmcp.bindingToken);
        router.push(playgroundPath);
        return { opened: true, path: playgroundPath };
      },
    }),
    [agent, agentId, playgroundPath, router, webmcp],
  );

  useWebMcpTool(openPlaygroundTool, {
    enabled: agent?.status === "active",
    scopeKey: agent?.id,
  });

  const handleExport = useCallback(
    async (format: "zip" | "markdown" = "zip") => {
      if (!agent || exporting) return;
      setExporting(true);
      setExportError(null);
      try {
        const blob =
          format === "zip"
            ? await exportAgentPackage(agent.name)
            : new Blob([await exportAgent.mutateAsync(agentId)], { type: "text/markdown" });
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = `${agent.name}.${format === "zip" ? "zip" : "md"}`;
        document.body.appendChild(link);
        link.click();
        document.body.removeChild(link);
        URL.revokeObjectURL(url);
      } catch (error) {
        setExportError(error instanceof Error ? error.message : "Export failed");
      } finally {
        setExporting(false);
      }
    },
    [agent, agentId, exportAgent, exporting],
  );

  const handleCopy = useCallback(async () => {
    try {
      const copied = await copyAgent.mutateAsync(agentId);
      router.push(`/agents/${copied.id}`);
    } catch (error) {
      console.error("Failed to copy agent:", error);
    }
  }, [agentId, copyAgent, router]);

  const handleConfirm = async () => {
    try {
      if (confirmAction === "archive") {
        await deleteAgent.mutateAsync(agentId);
        setConfirmAction(null);
      } else if (confirmAction === "delete") {
        await destroyAgent.mutateAsync(agentId);
        router.push("/agents");
      }
    } catch (error) {
      console.error(`Failed to ${confirmAction} agent:`, error);
    }
  };

  if (agentLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="mb-4 h-8 w-1/3" />
        <Skeleton className="mb-8 h-4 w-2/3" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!agent) {
    return (
      <ResourceNotFound
        title="Agent not found"
        description="This agent may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/agents"
        backLabel="Back to agents"
        resourceId={agentId}
      />
    );
  }

  const isActive = agent.status === "active";
  const sessionCount = agent.session_count;
  const displayName = getDisplayName(agent);
  const network = normalizeNetworkAccess(draft.networkAccess);
  const brandingParts = [
    draft.fields.description.trim() ? "Description" : "No description",
    ...(draft.starters.length > 0 ? [count(draft.starters.length, "starter")] : []),
  ];
  const moreRows: AgentMoreRow[] = [
    { id: "branding", label: "Branding", summary: brandingParts.join(" · ") },
    {
      id: "mcp",
      label: "MCP servers",
      summary: mcpAttachments?.length ? `${mcpAttachments.length} attached` : "None",
    },
    {
      id: "credentials",
      label: "Credentials",
      summary: credentials?.length ? `${credentials.length} bound` : "None",
    },
    {
      id: "files",
      label: "Starter files",
      summary: draft.files.length ? count(draft.files.length, "file") : "None",
    },
    {
      id: "network",
      label: "Network access",
      summary:
        network.allowed.length || network.blocked.length
          ? [
              ...(network.allowed.length ? [`${network.allowed.length} allowed`] : []),
              ...(network.blocked.length ? [`${network.blocked.length} blocked`] : []),
            ].join(" · ")
          : "Inherited",
    },
    {
      id: "environments",
      label: "Environments",
      summary: draft.environments
        ? `${Object.keys(draft.environments.profiles ?? {}).length} · default ${draft.environments.default}`
        : "None",
    },
    {
      id: "usage",
      label: "Token usage",
      summary: agent.usage
        ? formatTokens(agent.usage.input_tokens + agent.usage.output_tokens)
        : "None",
    },
    { id: "health", label: "Health check", summary: healthSummary(latestHealth) },
  ];

  const canArchive = isActive;
  const canDelete = agent.status === "archived" && can("agent.dangerous");
  const sheetSection = openSection === "versions" && !agentVersionsEnabled ? null : openSection;
  const confirmError = confirmAction === "delete" ? destroyAgent.error : deleteAgent.error;
  const confirmPending = deleteAgent.isPending || destroyAgent.isPending;

  const overflowMenu = (
    <DropdownMenu>
      <DropdownMenuTrigger
        className={buttonVariants({ variant: "outline", size: "icon" })}
        aria-label="More actions"
      >
        <MoreHorizontal className="size-4" />
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="end">
        <DropdownMenuContent>
          <DropdownMenuItem onClick={handleCopy} disabled={copyAgent.isPending}>
            <Copy className="size-4" />
            {copyAgent.isPending ? "Copying..." : "Copy"}
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => handleExport("zip")} disabled={exporting}>
            <Download className="size-4" />
            {exporting ? "Exporting..." : "Export package (ZIP)"}
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => handleExport("markdown")} disabled={exporting}>
            <Download className="size-4" />
            Export Markdown
          </DropdownMenuItem>
          {observersEnabled && isActive && (
            <DropdownMenuItem
              render={<Link href={{ pathname: "/observers/new", query: { agent_id: agentId } }} />}
            >
              <Telescope className="size-4" />
              Observe this agent
            </DropdownMenuItem>
          )}
          {agentVersionsEnabled && (
            <DropdownMenuItem onClick={() => setOpenSection("versions")}>
              <GitBranch className="size-4" />
              Version history
            </DropdownMenuItem>
          )}
          {(canArchive || canDelete) && <DropdownMenuSeparator />}
          {canArchive && (
            <DropdownMenuItem onClick={() => setConfirmAction("archive")}>
              <Archive className="size-4" />
              Archive agent
            </DropdownMenuItem>
          )}
          {canDelete && (
            <DropdownMenuItem
              onClick={() => setConfirmAction("delete")}
              className="text-destructive"
            >
              <Trash2 className="size-4" />
              Delete agent
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );

  const actions = editing ? (
    <>
      <Button onClick={handleSave} disabled={updateAgent.isPending}>
        <Check className="size-4" />
        {updateAgent.isPending ? "Saving..." : "Save changes"}
      </Button>
      <Button variant="outline" onClick={handleDiscard} disabled={updateAgent.isPending}>
        Discard
      </Button>
    </>
  ) : (
    <>
      {isActive && (
        <Button variant="outline" onClick={startEdit}>
          <Pencil className="size-4" />
          Edit
        </Button>
      )}
      {overflowMenu}
      <Button variant="accent" onClick={handleTestAgent} disabled={!isActive}>
        <MessageCircle className="size-4" />
        Test in Playground
      </Button>
    </>
  );

  return (
    <div className="min-h-full bg-brand-dots">
      <PageContainer>
        <PageBreadcrumb items={[{ label: "Agents", href: "/agents" }, { label: displayName }]} />

        <PageMasthead
          icon={<Boxes />}
          entityId={agent.id}
          title={<span className={getEntityNameClassName(agent.status)}>{displayName}</span>}
          badges={
            <>
              <span className="font-mono text-xs text-muted-foreground">{agent.name}</span>
              {editing ? (
                <Badge variant="accent">
                  <Pencil />
                  Editing
                </Badge>
              ) : (
                <Badge variant={getEntityStatusBadgeVariant(agent.status)}>{agent.status}</Badge>
              )}
            </>
          }
          description={
            editing
              ? "Changes apply to new sessions only. Running sessions keep the current definition."
              : undefined
          }
          actions={actions}
        />

        {exportError && (
          <p role="alert" className="text-destructive">
            Could not export: {exportError}
          </p>
        )}
        {updateAgent.error && (
          <p role="alert" className="-mt-2 text-sm text-destructive">
            Could not save: {updateAgent.error.message}
          </p>
        )}

        {/* The tab row and the workspace are one surface: no gap between them. */}
        <div className="flex min-w-0 flex-col">
          <PageControlStrip>
            <SectionTabs
              value={activeTab}
              onValueChange={(value) => {
                if (isAgentTab(value)) selectTab(value);
              }}
              items={getAgentTabItems(sessionCount)}
              className="border-x border-t bg-background px-2"
            />
          </PageControlStrip>

          {activeTab === "agent" && (
            <div className="grid min-w-0 border-x border-b bg-background lg:grid-cols-[minmax(0,1fr)_340px]">
              <AgentPromptPane
                className="lg:border-r"
                value={draft.fields.system_prompt}
                editing={editing}
                onEdit={isActive ? startEdit : undefined}
                onChange={(value) => draft.setField("system_prompt", value)}
                error={draft.errors.system_prompt}
                checks={
                  editing ? (
                    <AgentChecks
                      systemPrompt={draft.fields.system_prompt}
                      capabilities={draft.capabilities}
                      tools={agent.tools ?? []}
                      onApplyFix={(start, end, replacement) =>
                        draft.setField(
                          "system_prompt",
                          applyByteSpanReplacement(
                            draft.fields.system_prompt,
                            start,
                            end,
                            replacement,
                          ),
                        )
                      }
                    />
                  ) : undefined
                }
              />
              <div className="space-y-5">
                <AgentConfigColumn
                  className="border-t lg:border-t-0"
                  agent={agent}
                  draft={draft}
                  editing={editing}
                  readOnly={readOnly}
                  allCapabilities={allCapabilities ?? []}
                  defaultModel={defaultModel}
                  onStartEdit={startEdit}
                  moreRows={moreRows}
                  onOpenRow={(id) => setOpenSection(id as AgentSettingsSection)}
                />
                <AgentServiceAccount
                  agentId={agentId}
                  value={agent.service_virtual_user_id}
                  disabled={readOnly}
                />
              </div>
            </div>
          )}

          {activeTab !== "agent" && (
            <div className="min-w-0 pt-5 sm:pt-6">
              {activeTab === "preview" && (
                <AgentPreview
                  systemPrompt={draft.fields.system_prompt}
                  capabilities={draft.capabilities}
                  harnessId={draft.fields.harness_id || undefined}
                  mcpServers={agent.mcpServers}
                  initialFiles={draft.files}
                  tools={agent.tools ?? []}
                />
              )}
              {activeTab === "integrations" && <AgentIntegrationsPanel agent={agent} />}
              {activeTab === "stats" && (
                <ResourceStatsPanel stats={stats} isLoading={statsLoading} error={statsError} />
              )}
              {activeTab === "sessions" && <AgentSessionsPanel agentId={agentId} />}
            </div>
          )}
        </div>

        <PageFooter>
          <BackLink href="/agents">Back to Agents</BackLink>
        </PageFooter>
      </PageContainer>

      <AgentSettingsSheet
        section={sheetSection}
        onOpenChange={(open) => {
          if (!open) setOpenSection(null);
        }}
        agent={agent}
        draft={draft}
        readOnly={readOnly}
        onDraftChange={onDraftChange}
      />

      <Dialog
        open={confirmAction !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {confirmAction === "delete" ? "Delete agent" : "Archive agent"}
            </DialogTitle>
            <DialogDescription>
              {confirmAction === "delete"
                ? `Permanently delete the archived agent “${displayName}”? Existing references will render as deleted tombstones.`
                : `Archive “${displayName}”? It stays visible when archived items are shown, becomes read-only, and stops being assignable.`}
            </DialogDescription>
            {confirmError && confirmAction && (
              <EntityDeleteErrorNotice
                entityKind="agent"
                action={confirmAction}
                message={confirmError.message}
                className="mt-4"
              />
            )}
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmAction(null)}>
              Cancel
            </Button>
            <Button
              variant={confirmAction === "delete" ? "destructive" : "default"}
              onClick={handleConfirm}
              disabled={confirmPending}
            >
              {confirmAction === "delete"
                ? destroyAgent.isPending
                  ? "Deleting..."
                  : "Delete agent"
                : deleteAgent.isPending
                  ? "Archiving..."
                  : "Archive agent"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
