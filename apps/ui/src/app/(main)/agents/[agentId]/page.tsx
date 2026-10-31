"use client";

import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";

import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { ExternalWorker } from "@/components/agents/external-worker";
import { AgentSourcePanel } from "@/components/agents/agent-source";
import { useAgentSource } from "@/hooks/use-agent-source";
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
// Credentials are configuration (config column sheets). Copy, export, History,
// Manager notes, Archive and Delete live in the header EntityActionsMenu
// (knowledge/ui/entity-actions-menu.md); History replaces Version history, and
// old `?tab=versions` links open the History sheet.

import { use, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { Check, Copy, Download, MessageCircle, Pencil, RefreshCw, Telescope } from "lucide-react";
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
  useHarnesses,
  useLatestHealthCheckRun,
  useModels,
  usePageTitle,
  useUpdateAgent,
} from "@/hooks";
import { usePolicies } from "@/hooks/use-policies";
import { useVirtualUser } from "@/hooks/use-virtual-users";
import { useWebMcpTool } from "@/hooks/use-webmcp-tool";
import { ResourceNotFound } from "@/components/resource-not-found";
import { EntityDeleteErrorNotice } from "@/components/entity-delete-error-notice";
import {
  hasManagedSlackApps,
  SlackRemovalNotice,
} from "@/components/agents/integrations/slack-removal-notice";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
  EntityActionsMenu,
  EntityRecordSheets,
  entitySheetSearch,
} from "@/components/entity-actions/entity-actions-menu";
import { ChangeReasonField } from "@/components/entity-actions/change-reason-field";
import { ManagerNotesHint } from "@/components/entity-actions/manager-notes-hint";
import {
  BackLink,
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageFooter,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";
import { AgentAvatar } from "@/components/agents/agent-avatar";
import { AgentChecks, applyByteSpanReplacement } from "@/components/agents/agent-checks";
import { agentFilesSectionVisible } from "@/components/agents/agent-files-visibility";
import { AgentConfigColumn, type AgentMoreRow } from "@/components/agents/agent-config-column";
import { AgentCostEstimate } from "@/components/agents/agent-cost-estimate";
import { AgentIntegrationsPanel } from "@/components/agents/agent-integrations-panel";
import { useAgentChannels } from "@/hooks/use-agent-channels";
import { useAgentTriggers } from "@/hooks/use-agent-triggers";
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
import { harnessInheritsFromName } from "@/lib/harness-inheritance";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { useWebMcp } from "@/providers/webmcp-context";
import { AgentChatBoundary } from "@/components/chat-mode/agent-chat-boundary";
import { usePageChatMode } from "@/components/chat-mode/chat-mode-context";
import { chatPageRoute } from "@/lib/chat-page-context";
import { useAgentChatDraft } from "@/components/chat-mode/use-agent-chat-draft";
import {
  ChatChangeMark,
  ChatChangesContext,
  ChatChangesToast,
  ChatDraftChangesContext,
  ChatDraftChangesToast,
  useChatChanges,
} from "@/components/chat-mode/chat-change-mark";

/** Shown on the More row. Empty means the provider's own output limit. */
function maxOutputTokensSummary(value: string, locale: string, providerDefault: string): string {
  const trimmed = value.trim();
  if (!trimmed) return providerDefault;
  if (!/^\d+$/.test(trimmed)) return trimmed;
  return Number(trimmed).toLocaleString(locale);
}

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
  return (
    <AgentChatBoundary agentId={agentId}>
      <AgentDetailContent params={params} />
    </AgentChatBoundary>
  );
}

function AgentDetailContent({ params }: { params: Promise<{ agentId: string }> }) {
  const { displayLocale } = useDisplayLocale();
  const count = (n: number, singular: string, plural?: string) =>
    `${n} ${pluralize(n, singular, plural, displayLocale)}`;
  const ui = useUiTranslations();
  const { agentId } = use(params);
  const router = useRouter();
  const searchParams = useSearchParams();
  const tabParam = searchParams.get("tab");
  const deepLink = resolveAgentTab(tabParam);
  const [activeTab, setActiveTab] = useState<AgentTab>(deepLink.tab);
  useChatPageFilters({ tab: activeTab });
  const [openSection, setOpenSection] = useState<AgentSettingsSection | null>(deepLink.section);
  // Follow the address bar when it changes without a click (refresh already
  // lands here; back/forward and a replaced query must too).
  const [tabParamSeen, setTabParamSeen] = useState<string | null>(tabParam);
  if (tabParam !== tabParamSeen) {
    setTabParamSeen(tabParam);
    setActiveTab(deepLink.tab);
  }
  const [editRequested, setEditRequested] = useState(() => searchParams.get("mode") === "edit");
  // `?sync=github` reopens the Sync dialog after connecting GitHub in this tab
  // (the fallback when the browser blocks the connect popup).
  const [sourceOpen, setSourceOpen] = useState(() => searchParams.get("sync") === "github");
  const [confirmAction, setConfirmAction] = useState<"archive" | "delete" | null>(null);
  // Optional "Reason for this change", sent with Save and with Archive/Delete.
  const [saveReason, setSaveReason] = useState("");
  const [confirmReason, setConfirmReason] = useState("");

  // Retired `?tab=versions` links open the History sheet.
  useEffect(() => {
    if (!deepLink.recordSheet) return;
    const params = new URLSearchParams(searchParams.toString());
    params.delete("tab");
    router.replace(`/agents/${agentId}${entitySheetSearch(params, deepLink.recordSheet)}`, {
      scroll: false,
    });
  }, [agentId, deepLink.recordSheet, router, searchParams]);

  const observersEnabled = useFeatureFlag("observers");
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const sourceQuery = useAgentSource(agentId);
  const sourceManaged = !!sourceQuery.data && sourceQuery.data.mode !== "two_way";
  const { data: harnesses } = useHarnesses();
  usePageTitle(agent ? getDisplayName(agent, displayLocale) : null, ui("Agent"));
  const { data: allCapabilities } = useCapabilities({ includeRetired: true });
  const { data: models } = useModels();
  const { data: stats, isLoading: statsLoading, error: statsError } = useAgentStats(agentId);
  const { data: mcpAttachments } = useAgentMcpAttachments(agentId);
  const { data: credentials } = useAgentCredentials(agentId);
  const { data: serviceAccount } = useVirtualUser(agent?.service_virtual_user_id || undefined);
  const { data: latestHealth } = useLatestHealthCheckRun(agentId);
  const {
    data: channels,
    isError: channelsError,
    refetch: refetchChannels,
  } = useAgentChannels(agentId);
  const { data: triggers } = useAgentTriggers(agentId);
  const integrationCount = channels && triggers ? channels.length + triggers.length : undefined;
  const updateAgent = useUpdateAgent();
  const deleteAgent = useDeleteAgent();
  const destroyAgent = useDestroyAgent();
  const exportAgent = useExportAgent();
  const [exporting, setExporting] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);
  const copyAgent = useCopyAgent();
  const { can } = usePolicies("agents");
  const webmcp = useWebMcp();
  const chatMode = usePageChatMode();
  const chatChanges = useChatChanges();

  const draft = useAgentDraft(agent);
  // Built-in agents (Platform Chat) are managed by the platform: the API
  // rejects definition edits, archive and delete, so the page never offers
  // them. Bindings (service account, triggers, credentials) stay editable.
  const builtIn = !!agent?.is_built_in;
  const statusReadOnly = isReadOnlyStatus(agent?.status);
  const readOnly =
    builtIn || statusReadOnly || sourceManaged || sourceQuery.isLoading || sourceQuery.isError;
  const editing = editRequested && !!agent && !readOnly;
  const [draftConflict, setDraftConflict] = useState(false);
  const previousAgentUpdate = useRef(agent?.updated_at);
  const chatDraft = useAgentChatDraft({
    agentId,
    updatedAt: agent?.updated_at,
    draft,
    writable: !!agent && !readOnly && can("agent.manage"),
    blocked: draftConflict || updateAgent.isPending,
    beginEdit: () => setEditRequested(true),
  });
  useEffect(() => {
    if (
      chatMode &&
      editing &&
      draft.isDirty &&
      previousAgentUpdate.current &&
      previousAgentUpdate.current !== agent?.updated_at
    )
      setDraftConflict(true);
    previousAgentUpdate.current = agent?.updated_at;
  }, [agent?.updated_at, chatMode, draft.isDirty, editing]);
  const registerPage = chatMode?.registerPage;
  const pageRoute = chatPageRoute(`/agents/${agentId}`, searchParams);
  const pageLabel = agent ? getDisplayName(agent, displayLocale) : "Agent";
  const pageLoaded = !!agent;
  useEffect(
    () =>
      registerPage?.(
        {
          route: pageRoute,
          entity_type: "agent",
          entity_id: agentId,
          label: pageLabel,
          editing,
        },
        pageLoaded ? chatDraft.adapter : undefined,
      ),
    [registerPage, agentId, pageRoute, pageLabel, editing, pageLoaded, chatDraft.adapter],
  );

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

  const startEdit = () => {
    chatChanges?.dismiss();
    setEditRequested(true);
  };
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
    setDraftConflict(false);
    chatMode?.recordAction(`Discarded unsaved edits to ${pageLabel}`);
    draft.reset();
    chatDraft.changes.clear();
    updateAgent.reset();
    setSaveReason("");
    exitEdit();
  };

  const handleSave = async () => {
    if (draftConflict) return;
    const result = draft.buildRequest();
    if (!result.ok) {
      selectTab("agent");
      if (BRANDING_FIELDS.some((field) => result.errors[field])) setOpenSection("branding");
      else if (result.errors.max_output_tokens) setOpenSection("max_output_tokens");
      return;
    }
    try {
      await updateAgent.mutateAsync({
        agentId,
        request: result.request,
        reason: saveReason,
      });
      chatMode?.recordAction(`Saved page draft changes to ${pageLabel}`);
      draft.reset();
      chatDraft.changes.clear();
      setSaveReason("");
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
      annotations: {
        readOnlyHint: true,
        destructiveHint: false,
        idempotentHint: true,
      },
      execute: async () => {
        webmcp.assertBinding(webmcp.bindingToken);
        if (!agent || agent.id !== agentId || agent.status !== "active") {
          throw new DOMException("The bound agent is no longer active", "AbortError");
        }
        await webmcp.requestApproval({
          title: "Open this agent in Playground?",
          description: `Open Playground with ${getDisplayName(agent, displayLocale)} selected. No session, sandbox, or model usage starts until you confirm the Playground setup.`,
          confirmLabel: "Open Playground",
        });
        webmcp.assertBinding(webmcp.bindingToken);
        router.push(playgroundPath);
        return { opened: true, path: playgroundPath };
      },
    }),
    [agent, agentId, playgroundPath, router, webmcp, displayLocale],
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
            : new Blob([await exportAgent.mutateAsync(agentId)], {
                type: "text/markdown",
              });
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
        await deleteAgent.mutateAsync({ id: agentId, reason: confirmReason });
        await refetchChannels();
        setConfirmAction(null);
        setConfirmReason("");
      } else if (confirmAction === "delete") {
        await destroyAgent.mutateAsync({ id: agentId, reason: confirmReason });
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
        title={ui("Agent not found")}
        description={ui(
          "This agent may have been deleted, moved to another organization, or the URL may be wrong.",
        )}
        backHref="/agents"
        backLabel="Back to agents"
        resourceId={agentId}
      />
    );
  }

  const isActive = agent.status === "active";
  const canEdit = isActive && !readOnly;
  const sessionCount = agent.session_count;
  const displayName = getDisplayName(agent, displayLocale);
  const fixedSandbox = harnessInheritsFromName(
    draft.fields.harness_id || agent.harness_id,
    harnesses ?? [],
    "bashkit-worker",
  )
    ? "Bashkit Virtual Workspace"
    : undefined;
  const network = normalizeNetworkAccess(draft.networkAccess);
  const brandingParts = [
    ...(agent.avatar ? ["Avatar"] : []),
    draft.fields.description.trim() ? ui("Description") : ui("No description"),
    ...(draft.starters.length > 0 ? [count(draft.starters.length, "starter")] : []),
  ];
  // Keep an open Files sheet available if the draft drops the last file.
  const showFiles =
    agentFilesSectionVisible({
      capabilities: draft.capabilities,
      files: draft.files,
      harnessId: draft.fields.harness_id,
      harnesses,
      catalog: allCapabilities,
    }) || openSection === "files";
  const moreRows: AgentMoreRow[] = [
    {
      id: "localization",
      label: ui("Localization"),
      summary: Object.keys(draft.localizations).join(", ") || ui("Default"),
    },
    {
      id: "branding",
      label: ui("Branding"),
      summary: brandingParts.join(" · "),
    },
    {
      id: "mcp",
      label: ui("MCP servers"),
      summary: mcpAttachments?.length
        ? ui("{value0} attached", { value0: mcpAttachments.length })
        : ui("None"),
    },
    {
      id: "credentials",
      label: ui("Credentials"),
      summary: credentials?.length
        ? ui("{value0} bound", { value0: credentials.length })
        : ui("None"),
    },
    {
      id: "service",
      label: ui("Service account"),
      summary: agent.service_virtual_user_id
        ? (serviceAccount?.name ?? ui("Assigned"))
        : ui("On first use"),
    },
    {
      id: "files",
      label: ui("Files"),
      summary: draft.files.length ? count(draft.files.length, "file") : ui("None"),
    },
    {
      id: "network",
      label: ui("Network access"),
      summary:
        network.allowed.length || network.blocked.length
          ? [
              ...(network.allowed.length ? [`${network.allowed.length} allowed`] : []),
              ...(network.blocked.length ? [`${network.blocked.length} blocked`] : []),
            ].join(" · ")
          : ui("Inherited"),
    },
    {
      id: "sandbox",
      label: ui("Sandbox"),
      summary:
        fixedSandbox ??
        (draft.sandboxPolicy
          ? `${Object.keys(draft.sandboxPolicy.templates ?? {}).length} · default ${draft.sandboxPolicy.default}`
          : ui("None")),
    },
    {
      id: "max_output_tokens",
      label: ui("Max output tokens"),
      summary: maxOutputTokensSummary(
        draft.fields.max_output_tokens,
        displayLocale,
        ui("Provider default"),
      ),
    },
    {
      id: "usage",
      label: ui("Token usage"),
      summary: agent.usage
        ? formatTokens(agent.usage.input_tokens + agent.usage.output_tokens)
        : ui("None"),
    },
    ...(!builtIn ? [{ id: "tracing", label: ui("Tracing"), summary: ui("Destinations") }] : []),
    {
      id: "health",
      label: ui("Health check"),
      summary: ui(healthSummary(latestHealth)),
    },
  ].filter((row) => row.id !== "files" || showFiles);

  const canArchive = canEdit;
  const canDelete = !builtIn && agent.status === "archived" && can("agent.dangerous");
  const canManage = can("agent.manage");
  const confirmError = confirmAction === "delete" ? destroyAgent.error : deleteAgent.error;
  const confirmPending = deleteAgent.isPending || destroyAgent.isPending;
  const slackRemovalForbidden =
    !!channels && hasManagedSlackApps(channels) && !can("agent.dangerous");
  const recordPermissions = { manage: canManage };

  const overflowMenu = (
    <EntityActionsMenu
      entityRef={agent.id}
      kind="agent"
      entityName={displayName}
      permissions={recordPermissions}
      actions={[
        ...(!builtIn && canManage && !statusReadOnly && !sourceQuery.data
          ? [
              {
                id: "source",
                label: ui("Sync with GitHub"),
                icon: <RefreshCw className="size-4" />,
                onSelect: () => setSourceOpen(true),
                disabled: sourceQuery.isLoading || !!sourceQuery.error,
                disabledReason: sourceQuery.error
                  ? "Could not load Agent source"
                  : "Loading source",
              },
            ]
          : []),
        {
          id: "copy",
          label: copyAgent.isPending ? "Copying..." : "Copy",
          icon: <Copy className="size-4" />,
          onSelect: handleCopy,
          disabled: copyAgent.isPending,
        },
        {
          id: "export-zip",
          label: exporting ? "Exporting..." : "Export package (ZIP)",
          icon: <Download className="size-4" />,
          onSelect: () => handleExport("zip"),
          disabled: exporting,
          disabledReason: "An export is running",
        },
        {
          id: "export-markdown",
          label: ui("Export Markdown"),
          icon: <Download className="size-4" />,
          onSelect: () => handleExport("markdown"),
          disabled: exporting,
          disabledReason: "An export is running",
        },
        ...(observersEnabled && isActive
          ? [
              {
                id: "observe",
                label: ui("Observe this agent"),
                icon: <Telescope className="size-4" />,
                href: {
                  pathname: "/observers/new",
                  query: { agent_id: agentId },
                },
              },
            ]
          : []),
      ]}
      archive={canArchive ? { onSelect: () => setConfirmAction("archive") } : undefined}
      delete={canDelete ? { onSelect: () => setConfirmAction("delete") } : undefined}
    />
  );

  const actions = editing ? (
    <>
      <Button onClick={handleSave} disabled={updateAgent.isPending || draftConflict}>
        <Check className="size-4" />
        {updateAgent.isPending ? ui("Saving...") : ui("Save changes")}
      </Button>
      <Button variant="outline" onClick={handleDiscard} disabled={updateAgent.isPending}>
        {ui("Discard")}
      </Button>
    </>
  ) : (
    <>
      {canEdit && (
        <Button variant="outline" onClick={startEdit}>
          <Pencil className="size-4" />
          {ui("Edit")}
        </Button>
      )}
      {overflowMenu}
      <Button variant="accent" onClick={handleTestAgent} disabled={!isActive}>
        <MessageCircle className="size-4" />
        {ui("Test in Playground")}
      </Button>
    </>
  );

  return (
    <ChatDraftChangesContext.Provider value={chatDraft.changes}>
      <ChatChangesContext.Provider
        value={
          chatChanges
            ? {
                ...chatChanges,
                canUndo: canEdit,
                pending: chatChanges.pending || editing,
              }
            : null
        }
      >
        <div className="min-h-full bg-brand-dots">
          <PageContainer>
            <PageBreadcrumb
              items={[{ label: ui("Agents"), href: "/agents" }, { label: displayName }]}
            />

            <PageMasthead
              icon={<AgentAvatar avatar={agent.avatar} size={38} />}
              entityId={agent.id}
              title={<span className={getEntityNameClassName(agent.status)}>{displayName}</span>}
              badges={
                <>
                  <span className="font-mono text-xs text-muted-foreground">{agent.name}</span>
                  {editing ? (
                    <Badge variant="accent">
                      <Pencil />
                      {ui("Editing")}
                    </Badge>
                  ) : (
                    <Badge variant={getEntityStatusBadgeVariant(agent.status)}>
                      {ui(agent.status)}
                    </Badge>
                  )}
                  {builtIn && <Badge variant="outline">{ui("Built-in")}</Badge>}
                </>
              }
              description={
                editing
                  ? chatMode
                    ? ui(
                        "Unsaved draft. Save changes to update the Agent definition. Running turns continue.",
                      )
                    : ui(
                        "Changes apply to new sessions only. Running sessions keep the current definition.",
                      )
                  : builtIn
                    ? ui("Managed by Everruns and read-only. Copy it to make an editable version.")
                    : undefined
              }
              meta={
                editing ? (
                  <ManagerNotesHint entityRef={agent.id} kind="agent" enabled={canManage} />
                ) : undefined
              }
              actions={
                <>
                  <AgentCostEstimate
                    modelId={draft.fields.default_model_id}
                    maxOutputTokens={draft.fields.max_output_tokens}
                    previewRequest={{
                      system_prompt: draft.fields.system_prompt,
                      harness_id:
                        draft.fields.harness_id || agent.effective_harness?.id || undefined,
                      capabilities: draft.capabilities,
                      initial_files: draft.files,
                      tools: agent.tools ?? [],
                      mcpServers: agent.mcpServers,
                    }}
                  />
                  {actions}
                </>
              }
            />

            {!editing && !builtIn && (
              <ExternalWorker
                key={agent.id}
                agentId={agent.id}
                disabled={!can("agent.dangerous")}
              />
            )}

            {!editing && !builtIn && (
              <AgentSourcePanel
                agentId={agentId}
                disabled={statusReadOnly}
                open={sourceOpen}
                onOpenChange={setSourceOpen}
              />
            )}

            {editing && (
              <ChangeReasonField
                value={saveReason}
                onChange={setSaveReason}
                disabled={updateAgent.isPending}
                className="max-w-xl"
              />
            )}

            {exportError && (
              <p role="alert" className="text-destructive">
                {ui("Could not export:")} {exportError}
              </p>
            )}
            {updateAgent.error && (
              <p role="alert" className="-mt-2 text-sm text-destructive">
                {ui("Could not save:")} {updateAgent.error.message}
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
                  items={getAgentTabItems(sessionCount, integrationCount, ui)}
                  className="border-x border-t bg-background px-2"
                />
              </PageControlStrip>

              {activeTab === "agent" && (
                <div className="agent-workspace grid min-w-0 border-x border-b bg-background lg:grid-cols-[minmax(0,1fr)_340px]">
                  <ChatChangeMark field="system_prompt">
                    <AgentPromptPane
                      className="lg:border-r"
                      value={draft.fields.system_prompt}
                      editing={editing}
                      onEdit={canEdit ? startEdit : undefined}
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
                  </ChatChangeMark>
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
                  {activeTab === "integrations" && (
                    <AgentIntegrationsPanel
                      agent={agent}
                      initialChannelId={searchParams.get("channel")}
                    />
                  )}
                  {activeTab === "stats" && (
                    <ResourceStatsPanel stats={stats} isLoading={statsLoading} error={statsError} />
                  )}
                  {activeTab === "sessions" && (
                    <AgentSessionsPanel
                      agentId={agentId}
                      defaultModelId={agent?.default_model_id}
                    />
                  )}
                </div>
              )}
            </div>

            <PageFooter>
              <BackLink href="/agents">{ui("Back to Agents")}</BackLink>
            </PageFooter>
          </PageContainer>
          {draftConflict && (
            <p role="alert" className="mx-6 border border-accent bg-card p-3 text-sm">
              {ui(
                "The saved agent changed while you were editing. Your unsaved draft is kept. Discard it to review the latest definition before saving.",
              )}
            </p>
          )}
          <ChatChangesToast name={displayName} canUndo={canEdit} editing={editing} />
          <ChatDraftChangesToast />

          {/* The menu (and its sheets) is hidden in edit mode; `?sheet=` links such
          as the manager-notes hint still open there. */}
          {editing && (
            <EntityRecordSheets
              entityRef={agent.id}
              kind="agent"
              entityName={displayName}
              permissions={recordPermissions}
            />
          )}

          <AgentSettingsSheet
            section={openSection}
            onOpenChange={(open) => {
              if (!open) setOpenSection(null);
            }}
            agent={agent}
            draft={draft}
            readOnly={readOnly}
            fixedSandbox={fixedSandbox}
            onDraftChange={onDraftChange}
          />

          <Dialog
            open={confirmAction !== null}
            onOpenChange={(open) => {
              if (!open) {
                setConfirmAction(null);
                setConfirmReason("");
              }
            }}
          >
            <DialogContent>
              <DialogHeader>
                <DialogTitle>
                  {confirmAction === "delete" ? ui("Delete agent") : ui("Archive agent")}
                </DialogTitle>
                <DialogDescription>
                  {confirmAction === "delete"
                    ? ui(
                        "Permanently delete the archived agent “{value0}”? Existing references will render as deleted tombstones.",
                        { value0: displayName },
                      )
                    : ui(
                        "Archive “{value0}”? It stays visible when archived items are shown, becomes read-only, and stops being assignable.",
                        { value0: displayName },
                      )}
                </DialogDescription>
                {confirmAction && channels && (
                  <SlackRemovalNotice channels={channels} action={confirmAction} />
                )}
                {slackRemovalForbidden && (
                  <p className="text-sm text-muted-foreground">
                    {ui(
                      "Removing managed Slack apps requires permission to delete Agent integrations.",
                    )}
                  </p>
                )}
                {channels === undefined && (
                  <p className="text-sm text-muted-foreground">
                    {channelsError
                      ? ui("Could not check Slack connections. Reload before continuing.")
                      : ui("Checking Slack connections…")}
                  </p>
                )}
                {confirmError && confirmAction && (
                  <EntityDeleteErrorNotice
                    entityKind="agent"
                    action={confirmAction}
                    message={confirmError.message}
                    className="mt-4"
                  />
                )}
              </DialogHeader>
              <ChangeReasonField
                value={confirmReason}
                onChange={setConfirmReason}
                disabled={confirmPending}
              />
              <DialogFooter>
                <Button
                  variant="outline"
                  onClick={() => {
                    setConfirmAction(null);
                    setConfirmReason("");
                  }}
                >
                  {ui("Cancel")}
                </Button>
                <Button
                  variant={confirmAction === "delete" ? "destructive" : "default"}
                  onClick={handleConfirm}
                  disabled={confirmPending || channels === undefined || slackRemovalForbidden}
                >
                  {confirmAction === "delete"
                    ? destroyAgent.isPending
                      ? ui("Deleting...")
                      : ui("Delete agent")
                    : deleteAgent.isPending
                      ? ui("Archiving...")
                      : ui("Archive agent")}
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </div>
      </ChatChangesContext.Provider>
    </ChatDraftChangesContext.Provider>
  );
}
