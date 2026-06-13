"use client";

import { use, useState, useCallback } from "react";
import {
  useApp,
  useUpdateApp,
  useDeleteApp,
  useDestroyApp,
  usePublishApp,
  useUnpublishApp,
} from "@/hooks/use-apps";
import { useAgentVersions } from "@/hooks/use-agents";
import { usePolicies } from "@/hooks/use-policies";
import { useAgents, usePageTitle } from "@/hooks";
import { useHarnesses } from "@/hooks/use-harnesses";
import {
  getSlackManifest,
  updateChannel as apiUpdateChannel,
  addChannel as apiAddChannel,
  deleteChannel as apiDeleteChannel,
  addA2aChannel as apiAddA2aChannel,
  regenerateA2aChannelKey as apiRegenerateA2aChannelKey,
} from "@/lib/api/apps";
import { useRouter } from "next/navigation";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { queryKeys } from "@/lib/query-keys";
import Link from "next/link";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { AgentSelect } from "@/components/agent/agent-select";
import { AgentIdentitySelect } from "@/components/agent-identity/agent-identity-select";
import { HarnessSelect } from "@/components/harness/harness-select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  ArrowLeft,
  Globe,
  GlobeLock,
  Trash2,
  Pencil,
  Check,
  X,
  Rocket,
  Plus,
  RefreshCw,
} from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import { A2aAgentCardPreview } from "@/components/apps/a2a-agent-card-preview";
import { A2aSetupGuidance } from "@/components/apps/a2a-setup-guidance";
import { AgUiSetupGuidance } from "@/components/apps/ag-ui-setup-guidance";
import { FcpSetupGuidance } from "@/components/apps/fcp-setup-guidance";
import { AppBudgetsCard } from "@/components/apps/app-budgets-card";
import { AppDetailV2 } from "@/components/apps/app-detail-v2";
import { ScheduleSetupGuidance } from "@/components/apps/schedule-setup-guidance";
import { SlackSetupGuidance } from "@/components/apps/slack-setup-guidance";
import { WebhookSetupGuidance } from "@/components/apps/webhook-setup-guidance";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import type {
  A2aChannelConfig,
  AgUiChannelConfig,
  AgUiToolVisibility,
  AppChannel,
  AppEndpointAuthConfig,
  AppEndpointAuthMode,
  AgentVersionPolicy,
  ChannelType,
  FcpChannelConfig,
  InvocationSessionMode,
  ScheduleChannelConfig,
  SessionStrategy,
  SlackChannelConfig,
  SlackReplyMode,
  WebhookChannelConfig,
} from "@/lib/api/types";
import {
  DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
  DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS,
} from "@/lib/api/types/app-types";
import { generateChannelToken } from "@/lib/channel-tokens";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityReferenceClassName,
  getEntityReferenceLabel,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import {
  getAgUiToolVisibilityDisplayName,
  getChannelTypeDisplayName,
  getInvocationSessionModeDisplayName,
  getSessionStrategyDisplayName,
  getSlackReplyModeDisplayName,
} from "@/lib/app-channels";
import { cn } from "@/lib/utils";

type ChannelConfigInput =
  | SlackChannelConfig
  | AgUiChannelConfig
  | ScheduleChannelConfig
  | WebhookChannelConfig
  | A2aChannelConfig
  | FcpChannelConfig;

type ChannelSummary = {
  target: string;
  group: string;
  health: string;
  healthVariant: "default" | "secondary" | "destructive" | "outline";
};

function hasSlackCredentials(config?: SlackChannelConfig): boolean {
  return (
    !!(
      (config?.signing_secret && config.signing_secret.trim()) ||
      config?.signing_secret_configured
    ) && !!((config?.bot_token && config.bot_token.trim()) || config?.bot_token_configured)
  );
}

function hasToken(config?: AgUiChannelConfig | WebhookChannelConfig): boolean {
  return !!((config?.token && config.token.trim()) || config?.token_configured);
}

function hasA2aKey(config?: A2aChannelConfig): boolean {
  return !!(config?.api_key_hash || config?.api_key_prefix);
}

function authLabel(config?: ChannelConfigInput, fallback: string = "Public") {
  const auth = (config as { auth?: AppEndpointAuthConfig } | undefined)?.auth;
  switch (auth?.mode) {
    case "anonymous":
      return "Public";
    case "shared_secret":
      return "Token";
    case "api_key":
      return "API key";
    case "google_oidc":
      return "Google";
    case "oidc":
      return "OIDC";
    case "oauth2_introspection":
      return "OAuth2";
    case "http_basic":
      return "HTTP Basic";
    case "mtls":
      return "mTLS";
    default:
      return fallback;
  }
}

function splitList(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function authModeFromChannel(
  channelType: ChannelType,
  config?: ChannelConfigInput,
): AppEndpointAuthMode {
  const auth = (config as { auth?: AppEndpointAuthConfig } | undefined)?.auth;
  if (auth?.mode) return auth.mode;
  if (channelType === "a2a") return "api_key";
  if (channelType === "ag_ui" || channelType === "webhook") {
    return hasToken(config as AgUiChannelConfig | WebhookChannelConfig)
      ? "shared_secret"
      : "anonymous";
  }
  return "anonymous";
}

function getChannelSummary(channel: AppChannel, isPublished: boolean): ChannelSummary {
  const publishedPrefix = isPublished ? "" : "Draft: ";

  switch (channel.channel_type) {
    case "slack": {
      const config = channel.channel_config as SlackChannelConfig;
      const hasCredentials = hasSlackCredentials(config);
      const target = config?.channel_id ?? config?.team_id ?? "Workspace events";
      if (!hasCredentials) {
        return {
          target,
          group: "Client surfaces",
          health: "Needs credentials",
          healthVariant: "secondary",
        };
      }
      if (!config?.webhook_verified_at) {
        return {
          target,
          group: "Client surfaces",
          health: `${publishedPrefix}Needs webhook`,
          healthVariant: "secondary",
        };
      }
      if (!config?.first_message_received_at) {
        return {
          target,
          group: "Client surfaces",
          health: "Awaiting test",
          healthVariant: "outline",
        };
      }
      return {
        target,
        group: "Client surfaces",
        health: "Ready",
        healthVariant: "default",
      };
    }
    case "ag_ui": {
      const config = channel.channel_config as AgUiChannelConfig;
      return {
        target: hasToken(config) ? "Token-protected endpoint" : "Public client endpoint",
        group: "Client surfaces",
        health: isPublished ? "Ready" : "Draft",
        healthVariant: isPublished ? "default" : "secondary",
      };
    }
    case "schedule": {
      const config = channel.channel_config as ScheduleChannelConfig;
      const configured = !!config?.cron_expression && !!config?.message;
      return {
        target: config?.cron_expression
          ? `${config.cron_expression} (${config.timezone ?? "UTC"})`
          : "Cron trigger",
        group: "Automation",
        health: configured ? (isPublished ? "Active" : "Draft") : "Incomplete",
        healthVariant: configured && isPublished ? "default" : "secondary",
      };
    }
    case "webhook": {
      const config = channel.channel_config as WebhookChannelConfig;
      const configured = hasToken(config) && !!config?.message;
      return {
        target: "Authenticated HTTP endpoint",
        group: "Programmatic ingress",
        health: configured ? (isPublished ? "Ready" : "Draft") : "Incomplete",
        healthVariant: configured && isPublished ? "default" : "secondary",
      };
    }
    case "a2a": {
      const config = channel.channel_config as A2aChannelConfig;
      const configured = hasA2aKey(config) && !!config?.message;
      return {
        target: config?.agent_card_name || "Agent endpoint",
        group: "Programmatic ingress",
        health: configured ? (isPublished ? "Ready" : "Draft") : "Incomplete",
        healthVariant: configured && isPublished ? "default" : "secondary",
      };
    }
    case "fcp": {
      const config = channel.channel_config as FcpChannelConfig;
      const tokenConfigured = !!config?.token || !!config?.token_configured;
      const target = tokenConfigured
        ? "Token-protected text endpoint"
        : config?.anonymous === false
          ? "Restricted (no token configured)"
          : "Public text endpoint";
      const misconfigured = config?.anonymous === false && !tokenConfigured;
      return {
        target,
        group: "Client surfaces",
        health: misconfigured ? "Misconfigured" : isPublished ? "Ready" : "Draft",
        healthVariant: misconfigured ? "secondary" : isPublished ? "default" : "secondary",
      };
    }
  }
}

function ChannelFact({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="text-sm font-medium">{label}</p>
      <p className="text-sm text-muted-foreground">{value}</p>
    </div>
  );
}

function ChannelMessagePreview({ message }: { message: string }) {
  return (
    <div>
      <p className="text-sm font-medium">Invocation message</p>
      <div className="mt-2 rounded-md border p-3 text-sm text-muted-foreground">
        <code className="whitespace-pre-wrap break-words">{message || "Not set"}</code>
      </div>
    </div>
  );
}

function AppDetailPageLegacy({ params }: { params: Promise<{ appId: string }> }) {
  const { appId } = use(params);
  const router = useRouter();
  const queryClient = useQueryClient();
  const { data: app, isLoading } = useApp(appId);
  usePageTitle(app ? getDisplayName(app) : null, "App");
  const { data: agents } = useAgents({ includeArchived: true });
  const { data: harnesses } = useHarnesses({ includeArchived: true });
  const updateApp = useUpdateApp();
  const deleteAppMutation = useDeleteApp();
  const destroyAppMutation = useDestroyApp();
  const publishApp = usePublishApp();
  const unpublishApp = useUnpublishApp();
  const { can: canPolicies } = usePolicies("apps");
  const appBudgetsEnabled = useFeatureFlag("app_budgets");
  const agentVersionsEnabled = useFeatureFlag("agent_versions");

  const [editingBasic, setEditingBasic] = useState(false);
  const [editingChannelId, setEditingChannelId] = useState<string | null>(null);
  const [selectedChannelId, setSelectedChannelId] = useState<string | null>(null);
  const [channelDetailTab, setChannelDetailTab] = useState("overview");
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [showAddChannel, setShowAddChannel] = useState(false);
  const [editingChannelType, setEditingChannelType] = useState<ChannelType>("slack");
  const [addChannelType, setAddChannelType] = useState<ChannelType>("slack");

  // Basic info edit state
  const [editName, setEditName] = useState("");
  const [editDescription, setEditDescription] = useState("");
  const [editAgentId, setEditAgentId] = useState("");
  const [editAgentVersionPolicy, setEditAgentVersionPolicy] =
    useState<AgentVersionPolicy>("default");
  const [editAgentVersionId, setEditAgentVersionId] = useState("");
  const [editHarnessId, setEditHarnessId] = useState("");
  const [editAgentIdentityId, setEditAgentIdentityId] = useState("");

  // Channel config edit state (shared for edit and add)
  const [editSigningSecret, setEditSigningSecret] = useState("");
  const [editBotToken, setEditBotToken] = useState("");
  const [editChannelIdField, setEditChannelIdField] = useState("");
  const [editTeamId, setEditTeamId] = useState("");
  const [editSessionStrategy, setEditSessionStrategy] = useState<SessionStrategy>("per_thread");
  const [editReplyMode, setEditReplyMode] = useState<SlackReplyMode>("all_messages");
  const [editScheduleCronExpression, setEditScheduleCronExpression] = useState("0 * * * * * *");
  const [editScheduleTimezone, setEditScheduleTimezone] = useState("UTC");
  const [editInvocationSessionMode, setEditInvocationSessionMode] =
    useState<InvocationSessionMode>("shared_session");
  const [editChannelMessage, setEditChannelMessage] = useState("");
  const [editWebhookToken, setEditWebhookToken] = useState("");
  const [editAgUiExpirationHours, setEditAgUiExpirationHours] = useState(
    DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS / 3600,
  );
  const [editChannelEnabled, setEditChannelEnabled] = useState(true);
  const [editAgUiRateLimitPerMinute, setEditAgUiRateLimitPerMinute] = useState<string>("");
  const [editAgUiToken, setEditAgUiToken] = useState("");
  const [editAgUiToolVisibility, setEditAgUiToolVisibility] =
    useState<AgUiToolVisibility>("generic");
  const [editAgUiGenericToolText, setEditAgUiGenericToolText] = useState(
    DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
  );
  const [editA2aAgentCardName, setEditA2aAgentCardName] = useState("");
  const [editA2aAgentCardDescription, setEditA2aAgentCardDescription] = useState("");
  const [editA2aRateLimitPerMinute, setEditA2aRateLimitPerMinute] = useState<string>("");
  // FCP channel form state — kept narrow because FCP intentionally has a
  // small auth surface (anonymous + optional shared token) and no inline
  // AppEndpointAuthConfig modes.
  const [editFcpAnonymous, setEditFcpAnonymous] = useState(true);
  const [editFcpToken, setEditFcpToken] = useState("");
  const [editFcpHandshake, setEditFcpHandshake] = useState("");
  const [editFcpExpirationHours, setEditFcpExpirationHours] = useState(6);
  const [editFcpRateLimitPerMinute, setEditFcpRateLimitPerMinute] = useState<string>("");
  const [editFcpResponseTimeoutSeconds, setEditFcpResponseTimeoutSeconds] = useState<string>("120");
  const [editAuthMode, setEditAuthMode] = useState<AppEndpointAuthMode>("anonymous");
  const [editAuthGoogleClientId, setEditAuthGoogleClientId] = useState("");
  const [editAuthOidcIssuer, setEditAuthOidcIssuer] = useState("");
  const [editAuthIntrospectionUrl, setEditAuthIntrospectionUrl] = useState("");
  const [editAuthOAuthClientId, setEditAuthOAuthClientId] = useState("");
  const [editAuthOAuthClientSecret, setEditAuthOAuthClientSecret] = useState("");
  const [editAuthAudiences, setEditAuthAudiences] = useState("");
  const [editAuthScopes, setEditAuthScopes] = useState("");
  const [editAuthDomains, setEditAuthDomains] = useState("");
  const [editAuthBasicUsername, setEditAuthBasicUsername] = useState("");
  const [editAuthBasicPassword, setEditAuthBasicPassword] = useState("");
  const [editAuthBasicPasswordConfigured, setEditAuthBasicPasswordConfigured] = useState(false);
  const [editAuthMtlsHeader, setEditAuthMtlsHeader] = useState("x-forwarded-client-cert-subject");
  const [editAuthMtlsValues, setEditAuthMtlsValues] = useState("");
  const [editAuthMtlsProxySecretHeader, setEditAuthMtlsProxySecretHeader] =
    useState("x-proxy-secret");
  const [editAuthMtlsProxySecret, setEditAuthMtlsProxySecret] = useState("");
  const [editAuthMtlsProxySecretConfigured, setEditAuthMtlsProxySecretConfigured] = useState(false);

  // A2A: plaintext API key shown exactly once (after create or rotate). Cleared
  // when the operator dismisses the dialog.
  const [a2aRevealedKey, setA2aRevealedKey] = useState<{
    channelId: string;
    apiKey: string;
    title: string;
  } | null>(null);
  const [rotatingA2aChannelId, setRotatingA2aChannelId] = useState<string | null>(null);

  const [creatingSlackApp, setCreatingSlackApp] = useState(false);
  const { data: appAgentVersions = [] } = useAgentVersions(
    agentVersionsEnabled ? (app?.agent_id ?? undefined) : undefined,
  );
  const { data: editableAgentVersions = [] } = useAgentVersions(
    agentVersionsEnabled && editAgentId ? editAgentId : undefined,
  );
  const publishedEditableAgentVersions = editableAgentVersions.filter(
    (version) => version.is_published,
  );

  const invalidateApp = () => {
    queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(appId) });
    queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
  };

  const updateChannelMutation = useMutation({
    mutationFn: async ({
      channelId,
      config,
      enabled,
    }: {
      channelId: string;
      config: ChannelConfigInput;
      enabled: boolean;
    }) => {
      return apiUpdateChannel(appId, channelId, { channel_config: config, enabled });
    },
    onSuccess: invalidateApp,
  });

  const addChannelMutation = useMutation({
    mutationFn: async ({
      channelType,
      config,
      enabled,
    }: {
      channelType: ChannelType;
      config: ChannelConfigInput;
      enabled: boolean;
    }) => {
      return apiAddChannel(appId, {
        channel_type: channelType,
        channel_config: config,
        enabled,
      });
    },
    onSuccess: () => {
      invalidateApp();
      setShowAddChannel(false);
    },
  });

  const addA2aMutation = useMutation({
    mutationFn: async (input: {
      sessionMode: InvocationSessionMode;
      message: string;
      agentCardName?: string;
      agentCardDescription?: string;
      auth?: AppEndpointAuthConfig;
      enabled: boolean;
    }) => {
      return apiAddA2aChannel(appId, {
        session_mode: input.sessionMode,
        message: input.message,
        agent_card_name: input.agentCardName,
        agent_card_description: input.agentCardDescription,
        auth: input.auth,
        enabled: input.enabled,
      });
    },
    onSuccess: (response) => {
      invalidateApp();
      setShowAddChannel(false);
      setA2aRevealedKey({
        channelId: response.channel.id,
        apiKey: response.api_key,
        title: "A2A API key created",
      });
    },
  });

  const rotateA2aMutation = useMutation({
    mutationFn: async (channelId: string) => {
      setRotatingA2aChannelId(channelId);
      try {
        return await apiRegenerateA2aChannelKey(appId, channelId);
      } finally {
        setRotatingA2aChannelId(null);
      }
    },
    onSuccess: (response) => {
      invalidateApp();
      setA2aRevealedKey({
        channelId: response.channel.id,
        apiKey: response.api_key,
        title: "A2A API key rotated",
      });
    },
  });

  const deleteChannelMutation = useMutation({
    mutationFn: async (channelId: string) => {
      return apiDeleteChannel(appId, channelId);
    },
    onSuccess: invalidateApp,
  });

  const isPublished = app?.status === "published";
  const isArchived = app?.status === "archived";
  const isReadOnly = isReadOnlyStatus(app?.status);
  const canDangerousDelete = canPolicies("app.dangerous");

  const canPublishApp =
    app?.channels?.some((channel) => {
      if (!channel.enabled) {
        return false;
      }
      switch (channel.channel_type) {
        case "slack": {
          const config = channel.channel_config as SlackChannelConfig;
          return hasSlackCredentials(config);
        }
        case "ag_ui":
          return true;
        case "schedule": {
          const config = channel.channel_config as ScheduleChannelConfig;
          return !!config?.cron_expression && !!config?.message;
        }
        case "webhook": {
          const config = channel.channel_config as WebhookChannelConfig;
          return hasToken(config) && !!config?.message;
        }
        case "a2a": {
          const config = channel.channel_config as A2aChannelConfig;
          return hasA2aKey(config) && !!config?.message;
        }
        case "fcp": {
          const config = channel.channel_config as FcpChannelConfig;
          // FCP only requires a token when anonymous access is disabled.
          if (config?.anonymous === false) {
            return !!config?.token || !!config?.token_configured;
          }
          return true;
        }
      }
    }) ?? false;

  const slackWebhookUrl =
    typeof window !== "undefined"
      ? `${window.location.origin}/api/v1/apps/${appId}/slack/events`
      : `/api/v1/apps/${appId}/slack/events`;
  const agUiEndpointUrl =
    typeof window !== "undefined"
      ? `${window.location.origin}/api/v1/apps/${appId}/ag-ui`
      : `/api/v1/apps/${appId}/ag-ui`;
  const agUiImageUploadUrl = `${agUiEndpointUrl}/images`;
  const fcpEndpointUrl =
    typeof window !== "undefined"
      ? `${window.location.origin}/api/v1/apps/${appId}/fcp`
      : `/api/v1/apps/${appId}/fcp`;

  const isLocalhost =
    typeof window !== "undefined" &&
    (window.location.hostname === "localhost" || window.location.hostname === "127.0.0.1");
  const slackWebhookPath = `/api/v1/apps/${appId}/slack/events`;

  const handleCreateSlackApp = useCallback(async () => {
    setCreatingSlackApp(true);
    try {
      const manifest = await getSlackManifest(appId);
      if (manifest?.create_url) {
        window.open(manifest.create_url, "_blank");
      }
    } finally {
      setCreatingSlackApp(false);
    }
  }, [appId]);

  const startEditBasic = () => {
    if (!app) return;
    setEditName(app.name);
    setEditDescription(app.description ?? "");
    setEditAgentId(app.agent_id ?? "");
    setEditAgentVersionPolicy(app.agent_version_policy ?? "default");
    setEditAgentVersionId(app.agent_version_id ?? "");
    setEditHarnessId(app.harness_id);
    setEditAgentIdentityId(app.agent_identity_id ?? "");
    setEditingBasic(true);
  };

  const saveBasic = async () => {
    if (!app) return;
    await updateApp.mutateAsync({
      appId: app.id,
      data: {
        name: editName,
        description: editDescription || undefined,
        agent_id: editAgentId || undefined,
        ...(agentVersionsEnabled
          ? {
              agent_version_policy: editAgentId ? editAgentVersionPolicy : "default",
              agent_version_id:
                editAgentId && editAgentVersionPolicy === "pinned"
                  ? editAgentVersionId || null
                  : null,
            }
          : {}),
        agent_identity_id: editAgentIdentityId || null,
        harness_id: editHarnessId,
      },
    });
    setEditingBasic(false);
  };

  const resetChannelForm = (channelType: ChannelType) => {
    setEditingChannelType(channelType);
    setAddChannelType(channelType);
    setEditSigningSecret("");
    setEditBotToken("");
    setEditChannelIdField("");
    setEditTeamId("");
    setEditSessionStrategy("per_thread");
    setEditReplyMode("all_messages");
    setEditScheduleCronExpression("0 * * * * * *");
    setEditScheduleTimezone("UTC");
    setEditInvocationSessionMode("shared_session");
    setEditChannelMessage("");
    setEditWebhookToken("");
    setEditAgUiExpirationHours(DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS / 3600);
    setEditChannelEnabled(true);
    setEditAgUiRateLimitPerMinute("");
    setEditAgUiToken("");
    setEditAgUiToolVisibility("generic");
    setEditAgUiGenericToolText(DEFAULT_AG_UI_GENERIC_TOOL_TEXT);
    setEditA2aAgentCardName("");
    setEditA2aAgentCardDescription("");
    setEditA2aRateLimitPerMinute("");
    setEditFcpAnonymous(true);
    setEditFcpToken("");
    setEditFcpHandshake("");
    setEditFcpExpirationHours(6);
    setEditFcpRateLimitPerMinute("");
    setEditFcpResponseTimeoutSeconds("120");
    setEditAuthMode(channelType === "a2a" ? "api_key" : "anonymous");
    setEditAuthGoogleClientId("");
    setEditAuthOidcIssuer("");
    setEditAuthIntrospectionUrl("");
    setEditAuthOAuthClientId("");
    setEditAuthOAuthClientSecret("");
    setEditAuthAudiences("");
    setEditAuthScopes("");
    setEditAuthDomains("");
    setEditAuthBasicUsername("");
    setEditAuthBasicPassword("");
    setEditAuthBasicPasswordConfigured(false);
    setEditAuthMtlsHeader("x-forwarded-client-cert-subject");
    setEditAuthMtlsValues("");
    setEditAuthMtlsProxySecretHeader("x-proxy-secret");
    setEditAuthMtlsProxySecret("");
    setEditAuthMtlsProxySecretConfigured(false);
  };

  const buildEndpointAuthConfig = (): AppEndpointAuthConfig | undefined => {
    if (editAuthMode === "anonymous") {
      return { mode: "anonymous", requirements: {} };
    }
    if (editAuthMode === "shared_secret" || editAuthMode === "api_key") {
      return undefined;
    }
    const requirements = {
      ...(splitList(editAuthAudiences).length ? { audiences: splitList(editAuthAudiences) } : {}),
      ...(splitList(editAuthScopes).length ? { scopes: splitList(editAuthScopes) } : {}),
      ...(splitList(editAuthDomains).length ? { domains: splitList(editAuthDomains) } : {}),
    };
    if (editAuthMode === "google_oidc") {
      return {
        mode: "google_oidc",
        provider: {
          type: "google_oidc",
          client_id: editAuthGoogleClientId.trim(),
          allowed_domains: splitList(editAuthDomains),
        },
        requirements,
      };
    }
    if (editAuthMode === "oidc") {
      return {
        mode: "oidc",
        provider: { type: "oidc", issuer: editAuthOidcIssuer.trim() },
        requirements,
      };
    }
    if (editAuthMode === "oauth2_introspection") {
      return {
        mode: "oauth2_introspection",
        provider: {
          type: "oauth2_introspection",
          introspection_url: editAuthIntrospectionUrl.trim(),
          ...(editAuthOAuthClientId.trim() ? { client_id: editAuthOAuthClientId.trim() } : {}),
          ...(editAuthOAuthClientSecret ? { client_secret: editAuthOAuthClientSecret } : {}),
        },
        requirements,
      };
    }
    if (editAuthMode === "http_basic") {
      return {
        mode: "http_basic",
        provider: {
          type: "http_basic",
          username: editAuthBasicUsername.trim(),
          ...(editAuthBasicPassword ? { password: editAuthBasicPassword } : {}),
        },
        requirements: {},
      };
    }
    if (editAuthMode === "mtls") {
      return {
        mode: "mtls",
        provider: {
          type: "mtls",
          header_name: editAuthMtlsHeader.trim(),
          allowed_values: splitList(editAuthMtlsValues),
          proxy_secret_header: editAuthMtlsProxySecretHeader.trim(),
          ...(editAuthMtlsProxySecret ? { proxy_secret: editAuthMtlsProxySecret } : {}),
        },
        requirements: {},
      };
    }
    return undefined;
  };

  const buildChannelConfig = (channelType: ChannelType): ChannelConfigInput => {
    switch (channelType) {
      case "ag_ui": {
        const hours = Number.isFinite(editAgUiExpirationHours)
          ? Math.max(0, editAgUiExpirationHours)
          : DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS / 3600;
        const trimmed = editAgUiRateLimitPerMinute.trim();
        const parsed = trimmed === "" ? undefined : Number.parseInt(trimmed, 10);
        return {
          anonymous: editAuthMode === "anonymous" || editAuthMode === "shared_secret",
          ...(editAuthMode === "shared_secret" && editAgUiToken.trim()
            ? { token: editAgUiToken.trim() }
            : {}),
          ...(buildEndpointAuthConfig() ? { auth: buildEndpointAuthConfig() } : {}),
          session_expiration_seconds: Math.round(hours * 3600),
          tool_visibility: editAgUiToolVisibility,
          generic_tool_text: editAgUiGenericToolText.trim() || DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
          ...(parsed !== undefined && Number.isFinite(parsed) && parsed > 0
            ? { rate_limit_per_minute: parsed }
            : {}),
        };
      }
      case "schedule":
        return {
          cron_expression: editScheduleCronExpression,
          timezone: editScheduleTimezone || "UTC",
          session_mode: editInvocationSessionMode,
          message: editChannelMessage,
        };
      case "webhook":
        return {
          token: editWebhookToken,
          session_mode: editInvocationSessionMode,
          message: editChannelMessage,
        };
      case "a2a": {
        // Edits only touch non-secret fields. The server preserves the
        // existing `api_key_hash` and `api_key_prefix` across PATCH, so the
        // client must NOT send them. Use the dedicated rotate-key button to
        // issue a new key.
        const trimmedA2aRate = editA2aRateLimitPerMinute.trim();
        const parsedA2aRate =
          trimmedA2aRate === "" ? undefined : Number.parseInt(trimmedA2aRate, 10);
        return {
          session_mode: editInvocationSessionMode,
          message: editChannelMessage,
          agent_card_name: editA2aAgentCardName.trim() || undefined,
          agent_card_description: editA2aAgentCardDescription.trim() || undefined,
          ...(buildEndpointAuthConfig() ? { auth: buildEndpointAuthConfig() } : {}),
          ...(parsedA2aRate !== undefined && Number.isFinite(parsedA2aRate) && parsedA2aRate > 0
            ? { rate_limit_per_minute: parsedA2aRate }
            : {}),
        } as unknown as A2aChannelConfig;
      }
      case "slack":
        return {
          signing_secret: editSigningSecret,
          bot_token: editBotToken,
          session_strategy: editSessionStrategy,
          reply_mode: editReplyMode,
          ...(editChannelIdField ? { channel_id: editChannelIdField } : {}),
          ...(editTeamId ? { team_id: editTeamId } : {}),
        };
      case "fcp": {
        const hours = Number.isFinite(editFcpExpirationHours)
          ? Math.max(0, editFcpExpirationHours)
          : 6;
        const trimmed = editFcpRateLimitPerMinute.trim();
        const parsedRate = trimmed === "" ? undefined : Number.parseInt(trimmed, 10);
        const timeoutTrim = editFcpResponseTimeoutSeconds.trim();
        const parsedTimeout = timeoutTrim === "" ? 120 : Number.parseInt(timeoutTrim, 10);
        return {
          anonymous: editFcpAnonymous,
          ...(editFcpToken.trim() ? { token: editFcpToken.trim() } : {}),
          ...(editFcpHandshake.trim() ? { handshake: editFcpHandshake } : {}),
          session_expiration_seconds: Math.round(hours * 3600),
          ...(parsedRate !== undefined && Number.isFinite(parsedRate) && parsedRate > 0
            ? { rate_limit_per_minute: parsedRate }
            : {}),
          response_timeout_seconds:
            Number.isFinite(parsedTimeout) && parsedTimeout > 0 && parsedTimeout <= 600
              ? parsedTimeout
              : 120,
        };
      }
    }
  };

  const isChannelConfigValid = (channelType: ChannelType) => {
    const authValid =
      editAuthMode === "anonymous" ||
      editAuthMode === "shared_secret" ||
      editAuthMode === "api_key" ||
      (editAuthMode === "google_oidc" && !!editAuthGoogleClientId.trim()) ||
      (editAuthMode === "oidc" && !!editAuthOidcIssuer.trim()) ||
      (editAuthMode === "oauth2_introspection" && !!editAuthIntrospectionUrl.trim()) ||
      (editAuthMode === "http_basic" &&
        !!editAuthBasicUsername.trim() &&
        (!!editAuthBasicPassword || editAuthBasicPasswordConfigured)) ||
      (editAuthMode === "mtls" &&
        !!editAuthMtlsHeader.trim() &&
        splitList(editAuthMtlsValues).length > 0 &&
        !!editAuthMtlsProxySecretHeader.trim() &&
        (!!editAuthMtlsProxySecret || editAuthMtlsProxySecretConfigured));
    if (!authValid) return false;
    switch (channelType) {
      case "ag_ui": {
        if (editAuthMode === "shared_secret" && !editAgUiToken.trim()) {
          return false;
        }
        if (!Number.isFinite(editAgUiExpirationHours) || editAgUiExpirationHours < 0) {
          return false;
        }
        if (editAgUiToolVisibility === "generic" && editAgUiGenericToolText.trim().length > 120) {
          return false;
        }
        const trimmed = editAgUiRateLimitPerMinute.trim();
        if (trimmed === "") return true;
        const parsed = Number.parseInt(trimmed, 10);
        return (
          Number.isFinite(parsed) &&
          parsed >= 0 &&
          parsed <= 1_000_000 &&
          String(parsed) === trimmed
        );
      }
      case "schedule":
        return !!editScheduleCronExpression && !!editChannelMessage;
      case "webhook":
        return !!editWebhookToken && !!editChannelMessage;
      case "a2a": {
        if (!editChannelMessage) return false;
        const trimmedA2aRate = editA2aRateLimitPerMinute.trim();
        if (trimmedA2aRate === "") return true;
        const parsedA2aRate = Number.parseInt(trimmedA2aRate, 10);
        return (
          Number.isFinite(parsedA2aRate) &&
          parsedA2aRate >= 0 &&
          parsedA2aRate <= 1_000_000 &&
          String(parsedA2aRate) === trimmedA2aRate
        );
      }
      case "slack":
        return !!editSigningSecret && !!editBotToken;
      case "fcp": {
        if (!Number.isFinite(editFcpExpirationHours) || editFcpExpirationHours < 0) {
          return false;
        }
        const trimmed = editFcpRateLimitPerMinute.trim();
        if (trimmed !== "") {
          const parsed = Number.parseInt(trimmed, 10);
          if (!Number.isFinite(parsed) || parsed < 0 || parsed > 1_000_000) return false;
          if (String(parsed) !== trimmed) return false;
        }
        const timeoutTrim = editFcpResponseTimeoutSeconds.trim();
        if (timeoutTrim === "") return true;
        const parsedTimeout = Number.parseInt(timeoutTrim, 10);
        return (
          Number.isFinite(parsedTimeout) &&
          parsedTimeout > 0 &&
          parsedTimeout <= 600 &&
          String(parsedTimeout) === timeoutTrim
        );
      }
    }
  };

  const loadEndpointAuthConfig = (channelType: ChannelType, config: ChannelConfigInput) => {
    const auth = (config as { auth?: AppEndpointAuthConfig }).auth;
    setEditAuthMode(authModeFromChannel(channelType, config));
    setEditAuthAudiences(auth?.requirements?.audiences?.join(", ") ?? "");
    setEditAuthScopes(auth?.requirements?.scopes?.join(", ") ?? "");
    setEditAuthDomains(auth?.requirements?.domains?.join(", ") ?? "");
    setEditAuthGoogleClientId("");
    setEditAuthOidcIssuer("");
    setEditAuthIntrospectionUrl("");
    setEditAuthOAuthClientId("");
    setEditAuthOAuthClientSecret("");
    setEditAuthBasicUsername("");
    setEditAuthBasicPassword("");
    setEditAuthBasicPasswordConfigured(false);
    setEditAuthMtlsHeader("x-forwarded-client-cert-subject");
    setEditAuthMtlsValues("");
    setEditAuthMtlsProxySecretHeader("x-proxy-secret");
    setEditAuthMtlsProxySecret("");
    setEditAuthMtlsProxySecretConfigured(false);
    const provider = auth?.provider;
    if (provider?.type === "google_oidc") {
      setEditAuthGoogleClientId(provider.client_id ?? "");
      setEditAuthDomains(
        provider.allowed_domains?.join(", ") ?? auth?.requirements?.domains?.join(", ") ?? "",
      );
    } else if (provider?.type === "oidc") {
      setEditAuthOidcIssuer(provider.issuer ?? "");
    } else if (provider?.type === "oauth2_introspection") {
      setEditAuthIntrospectionUrl(provider.introspection_url ?? "");
      setEditAuthOAuthClientId(provider.client_id ?? "");
    } else if (provider?.type === "http_basic") {
      setEditAuthBasicUsername(provider.username ?? "");
      setEditAuthBasicPasswordConfigured(provider.password_configured === true);
    } else if (provider?.type === "mtls") {
      setEditAuthMtlsHeader(provider.header_name ?? "x-forwarded-client-cert-subject");
      setEditAuthMtlsValues(provider.allowed_values?.join(", ") ?? "");
      setEditAuthMtlsProxySecretHeader(provider.proxy_secret_header ?? "x-proxy-secret");
      setEditAuthMtlsProxySecretConfigured(provider.proxy_secret_configured === true);
    }
  };

  const startEditChannel = (channel: AppChannel) => {
    resetChannelForm(channel.channel_type);
    setEditChannelEnabled(channel.enabled);
    if (channel.channel_type === "ag_ui") {
      const config = channel.channel_config as AgUiChannelConfig;
      loadEndpointAuthConfig(channel.channel_type, config);
      const expSeconds =
        config?.session_expiration_seconds ?? DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS;
      setEditAgUiExpirationHours(expSeconds / 3600);
      setEditAgUiRateLimitPerMinute(
        config?.rate_limit_per_minute && config.rate_limit_per_minute > 0
          ? String(config.rate_limit_per_minute)
          : "",
      );
      setEditAgUiToken(config?.token ?? "");
      setEditAgUiToolVisibility(config?.tool_visibility ?? "generic");
      setEditAgUiGenericToolText(config?.generic_tool_text ?? DEFAULT_AG_UI_GENERIC_TOOL_TEXT);
    } else if (channel.channel_type === "slack") {
      const config = channel.channel_config as SlackChannelConfig;
      setEditSigningSecret(config?.signing_secret ?? "");
      setEditBotToken(config?.bot_token ?? "");
      setEditChannelIdField(config?.channel_id ?? "");
      setEditTeamId(config?.team_id ?? "");
      setEditSessionStrategy(config?.session_strategy ?? "per_thread");
      setEditReplyMode(config?.reply_mode ?? "all_messages");
    } else if (channel.channel_type === "schedule") {
      const config = channel.channel_config as ScheduleChannelConfig;
      setEditScheduleCronExpression(config?.cron_expression ?? "0 * * * * * *");
      setEditScheduleTimezone(config?.timezone ?? "UTC");
      setEditInvocationSessionMode(config?.session_mode ?? "shared_session");
      setEditChannelMessage(config?.message ?? "");
    } else if (channel.channel_type === "webhook") {
      const config = channel.channel_config as WebhookChannelConfig;
      setEditWebhookToken(config?.token ?? "");
      setEditInvocationSessionMode(config?.session_mode ?? "shared_session");
      setEditChannelMessage(config?.message ?? "");
    } else if (channel.channel_type === "a2a") {
      const config = channel.channel_config as A2aChannelConfig;
      loadEndpointAuthConfig(channel.channel_type, config);
      setEditInvocationSessionMode(config?.session_mode ?? "shared_session");
      setEditChannelMessage(config?.message ?? "");
      setEditA2aAgentCardName(config?.agent_card_name ?? "");
      setEditA2aAgentCardDescription(config?.agent_card_description ?? "");
      setEditA2aRateLimitPerMinute(
        config?.rate_limit_per_minute && config.rate_limit_per_minute > 0
          ? String(config.rate_limit_per_minute)
          : "",
      );
    } else if (channel.channel_type === "fcp") {
      const config = channel.channel_config as FcpChannelConfig;
      setEditFcpAnonymous(config?.anonymous ?? true);
      setEditFcpToken(config?.token ?? "");
      setEditFcpHandshake(config?.handshake ?? "");
      const expSeconds =
        typeof config?.session_expiration_seconds === "number"
          ? config.session_expiration_seconds
          : 6 * 60 * 60;
      setEditFcpExpirationHours(expSeconds / 3600);
      setEditFcpRateLimitPerMinute(
        config?.rate_limit_per_minute && config.rate_limit_per_minute > 0
          ? String(config.rate_limit_per_minute)
          : "",
      );
      setEditFcpResponseTimeoutSeconds(
        typeof config?.response_timeout_seconds === "number"
          ? String(config.response_timeout_seconds)
          : "120",
      );
    }
    setEditingChannelId(channel.id);
  };

  const startAddChannel = () => {
    resetChannelForm("slack");
    setShowAddChannel(true);
  };

  const saveChannel = async () => {
    if (!editingChannelId) return;
    await updateChannelMutation.mutateAsync({
      channelId: editingChannelId,
      config: buildChannelConfig(editingChannelType),
      enabled: editChannelEnabled,
    });
    setEditingChannelId(null);
  };

  const saveNewChannel = async () => {
    if (addChannelType === "a2a") {
      await addA2aMutation.mutateAsync({
        sessionMode: editInvocationSessionMode,
        message: editChannelMessage,
        agentCardName: editA2aAgentCardName.trim() || undefined,
        agentCardDescription: editA2aAgentCardDescription.trim() || undefined,
        auth: buildEndpointAuthConfig(),
        enabled: editChannelEnabled,
      });
      return;
    }
    await addChannelMutation.mutateAsync({
      channelType: addChannelType,
      config: buildChannelConfig(addChannelType),
      enabled: editChannelEnabled,
    });
  };

  const handleDelete = async () => {
    if (!app) return;
    await destroyAppMutation.mutateAsync(app.id);
    router.push("/apps");
  };

  const handleArchive = async () => {
    if (!app) return;
    await deleteAppMutation.mutateAsync(app.id);
  };

  const agent = agents?.find((candidate) => candidate.id === app?.agent_id);
  const harness = harnesses?.find((candidate) => candidate.id === app?.harness_id);
  const pinnedVersion = appAgentVersions.find(
    (version) => version.is_published && version.id === app?.agent_version_id,
  );

  if (isLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!app) {
    return (
      <ResourceNotFound
        title="App not found"
        description="This app may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/apps"
        backLabel="Back to apps"
        resourceId={appId}
      />
    );
  }

  const renderEndpointAuthFields = (channelType: ChannelType, formId: string) => {
    if (!["ag_ui", "a2a"].includes(channelType)) return null;
    return (
      <div className="space-y-3 rounded-md border p-3">
        <div className="space-y-1">
          <p className="text-sm font-medium">Auth</p>
          <p className="text-xs text-muted-foreground">
            Protect this App endpoint without creating a separate org-level provider.
          </p>
        </div>
        <div>
          <Label htmlFor={`endpoint_auth_${formId}`}>Mode</Label>
          <Select
            value={editAuthMode}
            onValueChange={(value) => setEditAuthMode(value as AppEndpointAuthMode)}
          >
            <SelectTrigger id={`endpoint_auth_${formId}`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="anonymous">Public</SelectItem>
              {channelType === "ag_ui" && (
                <SelectItem value="shared_secret">Shared token</SelectItem>
              )}
              {channelType === "a2a" && <SelectItem value="api_key">API key</SelectItem>}
              <SelectItem value="google_oidc">Google</SelectItem>
              <SelectItem value="oidc">OIDC</SelectItem>
              <SelectItem value="oauth2_introspection">OAuth2 introspection</SelectItem>
              <SelectItem value="http_basic">HTTP Basic</SelectItem>
              <SelectItem value="mtls">mTLS header</SelectItem>
            </SelectContent>
          </Select>
        </div>
        {editAuthMode === "google_oidc" && (
          <>
            <div>
              <Label htmlFor={`google_client_id_${formId}`}>Google client ID</Label>
              <Input
                id={`google_client_id_${formId}`}
                value={editAuthGoogleClientId}
                onChange={(event) => setEditAuthGoogleClientId(event.target.value)}
                placeholder="1234567890-abc.apps.googleusercontent.com"
              />
            </div>
            <div>
              <Label htmlFor={`google_domains_${formId}`}>Allowed domains</Label>
              <Input
                id={`google_domains_${formId}`}
                value={editAuthDomains}
                onChange={(event) => setEditAuthDomains(event.target.value)}
                placeholder="example.com, example.org"
              />
            </div>
          </>
        )}
        {editAuthMode === "oidc" && (
          <>
            <div>
              <Label htmlFor={`oidc_issuer_${formId}`}>Issuer URL</Label>
              <Input
                id={`oidc_issuer_${formId}`}
                value={editAuthOidcIssuer}
                onChange={(event) => setEditAuthOidcIssuer(event.target.value)}
                placeholder="https://auth.example.com"
              />
            </div>
            <div>
              <Label htmlFor={`oidc_audiences_${formId}`}>Audiences</Label>
              <Input
                id={`oidc_audiences_${formId}`}
                value={editAuthAudiences}
                onChange={(event) => setEditAuthAudiences(event.target.value)}
                placeholder="api://everruns-app"
              />
            </div>
          </>
        )}
        {(editAuthMode === "google_oidc" || editAuthMode === "oidc") && (
          <div>
            <Label htmlFor={`auth_scopes_${formId}`}>Required scopes</Label>
            <Input
              id={`auth_scopes_${formId}`}
              value={editAuthScopes}
              onChange={(event) => setEditAuthScopes(event.target.value)}
              placeholder="read:app, invoke:agent"
            />
          </div>
        )}
        {editAuthMode === "oauth2_introspection" && (
          <>
            <div>
              <Label htmlFor={`oauth2_introspection_url_${formId}`}>Introspection URL</Label>
              <Input
                id={`oauth2_introspection_url_${formId}`}
                value={editAuthIntrospectionUrl}
                onChange={(event) => setEditAuthIntrospectionUrl(event.target.value)}
                placeholder="https://auth.example.com/oauth2/introspect"
              />
            </div>
            <div>
              <Label htmlFor={`oauth2_client_id_${formId}`}>Client ID</Label>
              <Input
                id={`oauth2_client_id_${formId}`}
                value={editAuthOAuthClientId}
                onChange={(event) => setEditAuthOAuthClientId(event.target.value)}
                placeholder="optional"
              />
            </div>
            <div>
              <Label htmlFor={`oauth2_client_secret_${formId}`}>Client secret</Label>
              <Input
                id={`oauth2_client_secret_${formId}`}
                type="password"
                value={editAuthOAuthClientSecret}
                onChange={(event) => setEditAuthOAuthClientSecret(event.target.value)}
                placeholder="Leave blank to keep existing secret"
              />
            </div>
            <div>
              <Label htmlFor={`oauth2_scopes_${formId}`}>Required scopes</Label>
              <Input
                id={`oauth2_scopes_${formId}`}
                value={editAuthScopes}
                onChange={(event) => setEditAuthScopes(event.target.value)}
                placeholder="invoke:agent, read:app"
              />
            </div>
            <div>
              <Label htmlFor={`oauth2_audiences_${formId}`}>Audiences</Label>
              <Input
                id={`oauth2_audiences_${formId}`}
                value={editAuthAudiences}
                onChange={(event) => setEditAuthAudiences(event.target.value)}
                placeholder="api://everruns-app"
              />
            </div>
          </>
        )}
        {editAuthMode === "http_basic" && (
          <>
            <div>
              <Label htmlFor={`basic_username_${formId}`}>Username</Label>
              <Input
                id={`basic_username_${formId}`}
                value={editAuthBasicUsername}
                onChange={(event) => setEditAuthBasicUsername(event.target.value)}
              />
            </div>
            <div>
              <Label htmlFor={`basic_password_${formId}`}>Password</Label>
              <Input
                id={`basic_password_${formId}`}
                type="password"
                value={editAuthBasicPassword}
                onChange={(event) => setEditAuthBasicPassword(event.target.value)}
                placeholder={editingChannelId ? "Leave blank to keep existing password" : ""}
              />
            </div>
          </>
        )}
        {editAuthMode === "mtls" && (
          <>
            <div>
              <Label htmlFor={`mtls_header_${formId}`}>Verified client identity header</Label>
              <Input
                id={`mtls_header_${formId}`}
                value={editAuthMtlsHeader}
                onChange={(event) => setEditAuthMtlsHeader(event.target.value)}
              />
            </div>
            <div>
              <Label htmlFor={`mtls_values_${formId}`}>Allowed values</Label>
              <Input
                id={`mtls_values_${formId}`}
                value={editAuthMtlsValues}
                onChange={(event) => setEditAuthMtlsValues(event.target.value)}
                placeholder="CN=client,O=Example"
              />
            </div>
            <div>
              <Label htmlFor={`mtls_proxy_secret_header_${formId}`}>Proxy secret header</Label>
              <Input
                id={`mtls_proxy_secret_header_${formId}`}
                value={editAuthMtlsProxySecretHeader}
                onChange={(event) => setEditAuthMtlsProxySecretHeader(event.target.value)}
                placeholder="x-proxy-secret"
              />
            </div>
            <div>
              <Label htmlFor={`mtls_proxy_secret_${formId}`}>
                Proxy secret{editAuthMtlsProxySecretConfigured ? " (configured)" : ""}
              </Label>
              <Input
                id={`mtls_proxy_secret_${formId}`}
                type="password"
                value={editAuthMtlsProxySecret}
                onChange={(event) => setEditAuthMtlsProxySecret(event.target.value)}
                placeholder={
                  editAuthMtlsProxySecretConfigured ? "Leave blank to keep existing secret" : ""
                }
              />
            </div>
          </>
        )}
      </div>
    );
  };

  const renderChannelForm = (isSaving: boolean, formId: string = "default") => {
    const formChannelType = editingChannelId ? editingChannelType : addChannelType;
    const a2aEndpointPreviewUrl = editingChannelId
      ? `${typeof window !== "undefined" ? window.location.origin : ""}/api/v1/apps/${appId}/a2a/${editingChannelId}`
      : undefined;

    return (
      <div className="space-y-4">
        {!editingChannelId && (
          <div>
            <Label htmlFor={`channel_type_${formId}`}>Channel Type</Label>
            <Select
              value={addChannelType}
              onValueChange={(v) => {
                const nextChannelType = v as ChannelType;
                resetChannelForm(nextChannelType);
                if (nextChannelType === "ag_ui") {
                  setEditAgUiToken(generateChannelToken());
                }
              }}
            >
              <SelectTrigger id={`channel_type_${formId}`}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="slack">{getChannelTypeDisplayName("slack")}</SelectItem>
                <SelectItem value="ag_ui">{getChannelTypeDisplayName("ag_ui")}</SelectItem>
                <SelectItem value="schedule">{getChannelTypeDisplayName("schedule")}</SelectItem>
                <SelectItem value="webhook">{getChannelTypeDisplayName("webhook")}</SelectItem>
                <SelectItem value="a2a">{getChannelTypeDisplayName("a2a")}</SelectItem>
                <SelectItem value="fcp">{getChannelTypeDisplayName("fcp")}</SelectItem>
              </SelectContent>
            </Select>
          </div>
        )}

        <div className="flex items-center justify-between rounded-md border p-3">
          <div className="space-y-1">
            <p className="text-sm font-medium">Enabled</p>
            <p className="text-xs text-muted-foreground">
              Disabled channels stay attached to the app but do not accept or emit new invocations.
            </p>
          </div>
          <Switch checked={editChannelEnabled} onCheckedChange={setEditChannelEnabled} />
        </div>

        {formChannelType === "ag_ui" && (
          <>
            <div className="rounded-md border p-3 text-sm text-muted-foreground">
              Publish the app, then point an AG-UI client at the endpoint shown on this page.
            </div>

            {renderEndpointAuthFields(formChannelType, formId)}

            {editAuthMode === "shared_secret" && (
              <div>
                <Label htmlFor={`ag_ui_token_${formId}`}>Endpoint Token</Label>
                <div className="flex gap-2">
                  <Input
                    id={`ag_ui_token_${formId}`}
                    value={editAgUiToken}
                    onChange={(e) => setEditAgUiToken(e.target.value)}
                    className="font-mono"
                    placeholder="Generated bearer token"
                  />
                  <Button
                    type="button"
                    variant="outline"
                    size="icon"
                    onClick={() => setEditAgUiToken(generateChannelToken())}
                    aria-label="Regenerate AG-UI token"
                  >
                    <RefreshCw className="h-4 w-4" />
                  </Button>
                </div>
                <p className="mt-1 text-xs text-muted-foreground">
                  AG-UI clients must send this as `Authorization: Bearer &lt;token&gt;` or
                  `X-Everruns-AG-UI-Token`.
                </p>
              </div>
            )}

            <div>
              <Label htmlFor={`ag_ui_expiration_${formId}`}>Thread expiration (hours)</Label>
              <Input
                id={`ag_ui_expiration_${formId}`}
                type="number"
                min="0"
                step="0.25"
                value={editAgUiExpirationHours}
                onChange={(e) => setEditAgUiExpirationHours(Number(e.target.value))}
                placeholder="6"
              />
              <p className="mt-1 text-xs text-muted-foreground">
                After this window, requests reusing the same `threadId` are rejected with{" "}
                <code>410 Gone</code> and the client must start a new thread. Set to `0` to allow
                resumption indefinitely. Default is 6 hours.
              </p>
            </div>

            <div>
              <Label htmlFor={`ag_ui_rate_limit_${formId}`}>
                Rate limit (requests per minute, per IP)
              </Label>
              <Input
                id={`ag_ui_rate_limit_${formId}`}
                type="number"
                min={0}
                max={1000000}
                step={1}
                value={editAgUiRateLimitPerMinute}
                onChange={(e) => setEditAgUiRateLimitPerMinute(e.target.value)}
                placeholder="Leave blank to disable per-app cap"
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Throttles anonymous traffic to this app&apos;s AG-UI endpoint. Leave blank or set to
                0 to rely on the global API rate limit only. Maximum 1,000,000 per minute.
              </p>
            </div>

            <div>
              <Label htmlFor={`ag_ui_tool_visibility_${formId}`}>Tool activity</Label>
              <Select
                value={editAgUiToolVisibility}
                onValueChange={(value) => setEditAgUiToolVisibility(value as AgUiToolVisibility)}
              >
                <SelectTrigger id={`ag_ui_tool_visibility_${formId}`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">{getAgUiToolVisibilityDisplayName("none")}</SelectItem>
                  <SelectItem value="generic">
                    {getAgUiToolVisibilityDisplayName("generic")}
                  </SelectItem>
                  <SelectItem value="narrated">
                    {getAgUiToolVisibilityDisplayName("narrated")}
                  </SelectItem>
                </SelectContent>
              </Select>
              <p className="mt-1 text-xs text-muted-foreground">
                Controls public activity shown while tools run. Tool names, arguments, and results
                are never sent to AG-UI clients.
              </p>
            </div>

            {editAgUiToolVisibility === "generic" && (
              <div>
                <Label htmlFor={`ag_ui_generic_tool_text_${formId}`}>Generic activity text</Label>
                <Input
                  id={`ag_ui_generic_tool_text_${formId}`}
                  value={editAgUiGenericToolText}
                  onChange={(e) => setEditAgUiGenericToolText(e.target.value)}
                  maxLength={120}
                  placeholder={DEFAULT_AG_UI_GENERIC_TOOL_TEXT}
                />
                <p className="mt-1 text-xs text-muted-foreground">
                  Shown as transient public activity while the agent uses tools. Maximum 120
                  characters.
                </p>
              </div>
            )}
          </>
        )}

        {formChannelType === "slack" && (
          <>
            <div>
              <Label htmlFor={`signing_secret_${formId}`}>Signing Secret</Label>
              <Input
                id={`signing_secret_${formId}`}
                type="password"
                value={editSigningSecret}
                onChange={(e) => setEditSigningSecret(e.target.value)}
                placeholder="Your Slack app's signing secret"
                required
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Found in your Slack app &rarr; Settings &rarr; Basic Information &rarr; App
                Credentials
              </p>
            </div>

            <div>
              <Label htmlFor={`bot_token_${formId}`}>Bot User OAuth Token</Label>
              <Input
                id={`bot_token_${formId}`}
                type="password"
                value={editBotToken}
                onChange={(e) => setEditBotToken(e.target.value)}
                placeholder="xoxb-..."
                required
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Found in your Slack app &rarr; OAuth &amp; Permissions &rarr; Bot User OAuth Token
              </p>
            </div>

            <Separator />

            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label htmlFor={`team_id_${formId}`}>Workspace ID (optional)</Label>
                <Input
                  id={`team_id_${formId}`}
                  value={editTeamId}
                  onChange={(e) => setEditTeamId(e.target.value)}
                  placeholder="T0123456789"
                />
              </div>

              <div>
                <Label htmlFor={`channel_id_${formId}`}>Channel ID (optional)</Label>
                <Input
                  id={`channel_id_${formId}`}
                  value={editChannelIdField}
                  onChange={(e) => setEditChannelIdField(e.target.value)}
                  placeholder="C0123456789"
                />
              </div>
            </div>

            <div>
              <Label htmlFor={`session_strategy_${formId}`}>Session Strategy</Label>
              <Select
                value={editSessionStrategy}
                onValueChange={(v) => setEditSessionStrategy(v as SessionStrategy)}
              >
                <SelectTrigger id={`session_strategy_${formId}`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="per_thread">
                    {getSessionStrategyDisplayName("per_thread")}
                  </SelectItem>
                  <SelectItem value="per_channel">
                    {getSessionStrategyDisplayName("per_channel")}
                  </SelectItem>
                  <SelectItem value="per_user">
                    {getSessionStrategyDisplayName("per_user")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div>
              <Label htmlFor={`reply_mode_${formId}`}>Reply Mode</Label>
              <Select
                value={editReplyMode}
                onValueChange={(v) => setEditReplyMode(v as SlackReplyMode)}
              >
                <SelectTrigger id={`reply_mode_${formId}`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all_messages">
                    {getSlackReplyModeDisplayName("all_messages")}
                  </SelectItem>
                  <SelectItem value="report_progress_only">
                    {getSlackReplyModeDisplayName("report_progress_only")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
          </>
        )}

        {formChannelType === "schedule" && (
          <>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label htmlFor={`schedule_cron_${formId}`}>Cron Expression</Label>
                <Input
                  id={`schedule_cron_${formId}`}
                  value={editScheduleCronExpression}
                  onChange={(e) => setEditScheduleCronExpression(e.target.value)}
                  placeholder="0 * * * * * *"
                />
              </div>
              <div>
                <Label htmlFor={`schedule_timezone_${formId}`}>Timezone</Label>
                <Input
                  id={`schedule_timezone_${formId}`}
                  value={editScheduleTimezone}
                  onChange={(e) => setEditScheduleTimezone(e.target.value)}
                  placeholder="UTC"
                />
              </div>
            </div>
            <div>
              <Label htmlFor={`schedule_session_mode_${formId}`}>Invocation Session Mode</Label>
              <Select
                value={editInvocationSessionMode}
                onValueChange={(v) => setEditInvocationSessionMode(v as InvocationSessionMode)}
              >
                <SelectTrigger id={`schedule_session_mode_${formId}`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="shared_session">
                    {getInvocationSessionModeDisplayName("shared_session")}
                  </SelectItem>
                  <SelectItem value="session_per_invocation">
                    {getInvocationSessionModeDisplayName("session_per_invocation")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label htmlFor={`schedule_message_${formId}`}>Invocation Message</Label>
              <Textarea
                id={`schedule_message_${formId}`}
                value={editChannelMessage}
                onChange={(e) => setEditChannelMessage(e.target.value)}
                placeholder="Run repository checks for {{app.name}}"
              />
            </div>
          </>
        )}

        {formChannelType === "webhook" && (
          <>
            <div>
              <Label htmlFor={`webhook_token_${formId}`}>Webhook Token</Label>
              <Input
                id={`webhook_token_${formId}`}
                type="password"
                value={editWebhookToken}
                onChange={(e) => setEditWebhookToken(e.target.value)}
                placeholder="shared-secret"
              />
            </div>
            <div>
              <Label htmlFor={`webhook_session_mode_${formId}`}>Invocation Session Mode</Label>
              <Select
                value={editInvocationSessionMode}
                onValueChange={(v) => setEditInvocationSessionMode(v as InvocationSessionMode)}
              >
                <SelectTrigger id={`webhook_session_mode_${formId}`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="shared_session">
                    {getInvocationSessionModeDisplayName("shared_session")}
                  </SelectItem>
                  <SelectItem value="session_per_invocation">
                    {getInvocationSessionModeDisplayName("session_per_invocation")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label htmlFor={`webhook_message_${formId}`}>Invocation Message</Label>
              <Textarea
                id={`webhook_message_${formId}`}
                value={editChannelMessage}
                onChange={(e) => setEditChannelMessage(e.target.value)}
                placeholder="Process webhook payload for {{payload.repo.name}}"
              />
            </div>
          </>
        )}

        {formChannelType === "a2a" && (
          <>
            <div className="rounded-md border p-3 text-sm text-muted-foreground">
              The A2A channel exposes this app over the Agent2Agent JSON-RPC protocol. The API key
              is generated on save and shown once — store it before closing the dialog. Rotate it
              later from the channel card if it leaks.
            </div>
            {renderEndpointAuthFields(formChannelType, formId)}
            <div>
              <Label htmlFor={`a2a_session_mode_${formId}`}>Invocation Session Mode</Label>
              <Select
                value={editInvocationSessionMode}
                onValueChange={(v) => setEditInvocationSessionMode(v as InvocationSessionMode)}
              >
                <SelectTrigger id={`a2a_session_mode_${formId}`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="shared_session">
                    {getInvocationSessionModeDisplayName("shared_session")}
                  </SelectItem>
                  <SelectItem value="session_per_invocation">
                    {getInvocationSessionModeDisplayName("session_per_invocation")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label htmlFor={`a2a_message_${formId}`}>Invocation Message</Label>
              <Textarea
                id={`a2a_message_${formId}`}
                value={editChannelMessage}
                onChange={(e) => setEditChannelMessage(e.target.value)}
                placeholder="A2A request: {{a2a.text}}"
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Templates can reference <code>{"{{a2a.text}}"}</code>, <code>{"{{payload}}"}</code>,
                and app metadata.
              </p>
            </div>
            <div>
              <Label htmlFor={`a2a_card_name_${formId}`}>Agent Card name (optional)</Label>
              <Input
                id={`a2a_card_name_${formId}`}
                value={editA2aAgentCardName}
                onChange={(e) => setEditA2aAgentCardName(e.target.value)}
                placeholder={app?.name ?? ""}
              />
            </div>
            <div>
              <Label htmlFor={`a2a_card_description_${formId}`}>
                Agent Card description (optional)
              </Label>
              <Textarea
                id={`a2a_card_description_${formId}`}
                value={editA2aAgentCardDescription}
                onChange={(e) => setEditA2aAgentCardDescription(e.target.value)}
                placeholder="What other agents should know about this app."
              />
            </div>
            <A2aAgentCardPreview
              appName={app?.name ?? "App"}
              appDescription={app?.description}
              endpointUrl={a2aEndpointPreviewUrl}
              agentCardName={editA2aAgentCardName}
              agentCardDescription={editA2aAgentCardDescription}
              sessionMode={editInvocationSessionMode}
            />
            <div>
              <Label htmlFor={`a2a_rate_limit_${formId}`}>
                Per-IP rate limit (requests/minute, optional)
              </Label>
              <Input
                id={`a2a_rate_limit_${formId}`}
                type="number"
                inputMode="numeric"
                min={0}
                max={1_000_000}
                value={editA2aRateLimitPerMinute}
                onChange={(e) => setEditA2aRateLimitPerMinute(e.target.value)}
                placeholder="No per-channel limit"
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Caps unattended A2A traffic per source IP. Leave empty (or set to 0) to disable —
                the global API limit still applies.
              </p>
            </div>
          </>
        )}

        {formChannelType === "fcp" && (
          <>
            <div className="rounded-md border p-3 text-sm text-muted-foreground">
              FCP is a minimal text-first endpoint. POST plain text (or JSON{" "}
              <code>&#123;&quot;message&quot;: &quot;...&quot;&#125;</code>) and receive the
              agent&apos;s Markdown reply. The handshake is the GET on the endpoint URL — keep it
              accurate so external clients can discover how to use this app.
            </div>
            <div className="flex items-center justify-between border p-3">
              <div>
                <Label htmlFor={`fcp_anonymous_${formId}`}>Anonymous access</Label>
                <p className="mt-1 text-xs text-muted-foreground">
                  When off, every POST must present the bearer token below. The GET handshake stays
                  open either way.
                </p>
              </div>
              <Switch
                id={`fcp_anonymous_${formId}`}
                checked={editFcpAnonymous}
                onCheckedChange={setEditFcpAnonymous}
              />
            </div>
            <div>
              <Label htmlFor={`fcp_token_${formId}`}>Bearer token (optional)</Label>
              <div className="flex gap-2">
                <Input
                  id={`fcp_token_${formId}`}
                  value={editFcpToken}
                  onChange={(e) => setEditFcpToken(e.target.value)}
                  className="font-mono"
                  placeholder="Leave blank for anonymous access"
                />
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  onClick={() => setEditFcpToken(generateChannelToken())}
                  aria-label="Regenerate FCP token"
                >
                  <RefreshCw className="w-4 h-4" />
                </Button>
              </div>
              <p className="mt-1 text-xs text-muted-foreground">
                Verified with constant-time comparison. Sent as Authorization: Bearer&nbsp;
                &lt;token&gt; or X-Everruns-FCP-Token. Never echoed in responses.
              </p>
            </div>
            <div>
              <Label htmlFor={`fcp_handshake_${formId}`}>Custom handshake (Markdown)</Label>
              <Textarea
                id={`fcp_handshake_${formId}`}
                value={editFcpHandshake}
                onChange={(e) => setEditFcpHandshake(e.target.value)}
                placeholder="Leave blank to auto-generate from the app name and description."
                rows={4}
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Returned verbatim for GET on the endpoint URL. Should describe what this app does
                and how to talk to it. Maximum 8 KiB.
              </p>
            </div>
            <div>
              <Label htmlFor={`fcp_expiration_${formId}`}>Session expiration (hours)</Label>
              <Input
                id={`fcp_expiration_${formId}`}
                type="number"
                inputMode="numeric"
                min={0}
                step={0.1}
                value={editFcpExpirationHours}
                onChange={(e) => setEditFcpExpirationHours(Number.parseFloat(e.target.value) || 0)}
              />
              <p className="mt-1 text-xs text-muted-foreground">
                0 = the fcp_session cookie never expires. Default is 6 hours.
              </p>
            </div>
            <div>
              <Label htmlFor={`fcp_rate_limit_${formId}`}>
                Per-IP rate limit (requests/minute, optional)
              </Label>
              <Input
                id={`fcp_rate_limit_${formId}`}
                type="number"
                inputMode="numeric"
                min={0}
                max={1_000_000}
                value={editFcpRateLimitPerMinute}
                onChange={(e) => setEditFcpRateLimitPerMinute(e.target.value)}
                placeholder="No per-channel limit"
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Counted in an FCP-only limiter namespace — cannot be shared with any other channel.
                Leave empty (or set to 0) to disable; the global API limit still applies.
              </p>
            </div>
            <div>
              <Label htmlFor={`fcp_response_timeout_${formId}`}>Response timeout (seconds)</Label>
              <Input
                id={`fcp_response_timeout_${formId}`}
                type="number"
                inputMode="numeric"
                min={1}
                max={600}
                value={editFcpResponseTimeoutSeconds}
                onChange={(e) => setEditFcpResponseTimeoutSeconds(e.target.value)}
                placeholder="120"
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Maximum time the endpoint waits for the agent before returning 504. Must be 1-600s.
                Default 120s. The session cookie is still set on timeout so the client can retry the
                same conversation.
              </p>
            </div>
          </>
        )}

        <div className="flex gap-2 pt-2">
          <Button
            size="sm"
            onClick={editingChannelId ? saveChannel : saveNewChannel}
            disabled={isSaving || !isChannelConfigValid(formChannelType)}
          >
            <Check className="w-3 h-3 mr-1" />
            {isSaving ? "Saving..." : "Save"}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setEditingChannelId(null);
              setShowAddChannel(false);
            }}
          >
            <X className="w-3 h-3 mr-1" />
            Cancel
          </Button>
        </div>
      </div>
    );
  };

  const renderChannelDisplay = (channel: AppChannel) => {
    if (channel.channel_type === "ag_ui") {
      const config = channel.channel_config as AgUiChannelConfig;
      return (
        <div key={channel.id} className="space-y-4">
          <AgUiSetupGuidance
            endpointUrl={agUiEndpointUrl}
            imageUploadUrl={agUiImageUploadUrl}
            isPublished={isPublished}
            anonymousEnabled={config?.anonymous ?? true}
            sessionExpirationSeconds={
              config?.session_expiration_seconds ?? DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS
            }
            rateLimitPerMinute={config?.rate_limit_per_minute}
            token={config?.token}
            tokenConfigured={hasToken(config)}
            toolVisibility={config?.tool_visibility ?? "generic"}
            genericToolText={config?.generic_tool_text ?? DEFAULT_AG_UI_GENERIC_TOOL_TEXT}
            onConfigure={() => startEditChannel(channel)}
          />
        </div>
      );
    }

    if (channel.channel_type === "schedule") {
      const config = channel.channel_config as ScheduleChannelConfig;
      return (
        <div key={channel.id} className="space-y-4">
          <ScheduleSetupGuidance
            cronExpression={config?.cron_expression ?? ""}
            timezone={config?.timezone ?? "UTC"}
            sessionMode={config?.session_mode ?? "shared_session"}
            message={config?.message ?? ""}
            isPublished={isPublished}
            onConfigure={() => startEditChannel(channel)}
          />
        </div>
      );
    }

    if (channel.channel_type === "fcp") {
      const config = channel.channel_config as FcpChannelConfig;
      return (
        <div key={channel.id} className="space-y-4">
          <FcpSetupGuidance
            endpointUrl={fcpEndpointUrl}
            isPublished={isPublished}
            anonymousEnabled={config?.anonymous ?? true}
            sessionExpirationSeconds={
              typeof config?.session_expiration_seconds === "number"
                ? config.session_expiration_seconds
                : 6 * 60 * 60
            }
            rateLimitPerMinute={config?.rate_limit_per_minute}
            responseTimeoutSeconds={config?.response_timeout_seconds}
            token={config?.token}
            tokenConfigured={!!config?.token || !!config?.token_configured}
            hasCustomHandshake={!!config?.handshake?.trim()}
            onConfigure={() => startEditChannel(channel)}
          />
        </div>
      );
    }

    if (channel.channel_type === "webhook") {
      const config = channel.channel_config as WebhookChannelConfig;
      const endpointUrl =
        typeof window !== "undefined"
          ? `${window.location.origin}/api/v1/apps/${appId}/webhooks/${channel.id}`
          : `/api/v1/apps/${appId}/webhooks/${channel.id}`;

      return (
        <div key={channel.id} className="space-y-4">
          <WebhookSetupGuidance
            endpointUrl={endpointUrl}
            sessionMode={config?.session_mode ?? "shared_session"}
            message={config?.message ?? ""}
            tokenConfigured={hasToken(config)}
            isPublished={isPublished}
            onConfigure={() => startEditChannel(channel)}
          />
        </div>
      );
    }

    if (channel.channel_type === "a2a") {
      const config = channel.channel_config as A2aChannelConfig;
      const baseUrl = typeof window !== "undefined" ? window.location.origin : "";
      const endpointUrl = `${baseUrl}/api/v1/apps/${appId}/a2a/${channel.id}`;
      const agentCardUrl = `${baseUrl}/api/v1/apps/${appId}/a2a/${channel.id}/.well-known/agent-card.json`;

      return (
        <div key={channel.id} className="space-y-4">
          <A2aSetupGuidance
            endpointUrl={endpointUrl}
            agentCardUrl={agentCardUrl}
            apiKeyPrefix={config?.api_key_prefix ?? ""}
            sessionMode={config?.session_mode ?? "shared_session"}
            message={config?.message ?? ""}
            appName={app.name}
            agentCardName={config?.agent_card_name}
            agentCardDescription={config?.agent_card_description}
            isPublished={isPublished}
            channelEnabled={channel.enabled}
            onConfigure={() => startEditChannel(channel)}
            onRotateKey={isReadOnly ? undefined : () => rotateA2aMutation.mutate(channel.id)}
            isRotating={rotateA2aMutation.isPending && rotatingA2aChannelId === channel.id}
          />
        </div>
      );
    }

    const config = channel.channel_config as SlackChannelConfig;
    const chHasConfig = hasSlackCredentials(config);
    const chWebhookVerified = !!config?.webhook_verified_at;
    const chFirstMsg = !!config?.first_message_received_at;

    return (
      <div key={channel.id} className="space-y-4">
        {chHasConfig ? (
          <>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <p className="text-sm font-medium">Signing Secret</p>
                <p className="text-sm text-muted-foreground font-mono">{"*".repeat(12)}</p>
              </div>
              <div>
                <p className="text-sm font-medium">Bot User OAuth Token</p>
                <p className="text-sm text-muted-foreground font-mono">{"*".repeat(12)}</p>
              </div>
            </div>

            {(config?.team_id || config?.channel_id) && (
              <div className="grid grid-cols-2 gap-4">
                {config?.team_id && (
                  <div>
                    <p className="text-sm font-medium">Workspace ID</p>
                    <p className="text-sm text-muted-foreground font-mono">{config.team_id}</p>
                  </div>
                )}
                {config?.channel_id && (
                  <div>
                    <p className="text-sm font-medium">Channel ID</p>
                    <p className="text-sm text-muted-foreground font-mono">{config.channel_id}</p>
                  </div>
                )}
              </div>
            )}

            <div>
              <p className="text-sm font-medium">Session Strategy</p>
              <p className="text-sm text-muted-foreground">
                {config?.session_strategy === "per_thread"
                  ? "Per Thread"
                  : config?.session_strategy === "per_channel"
                    ? "Per Channel"
                    : "Per User"}
              </p>
            </div>

            <div>
              <p className="text-sm font-medium">Reply Mode</p>
              <p className="text-sm text-muted-foreground">
                {config?.reply_mode === "report_progress_only"
                  ? "Report Progress Only"
                  : "All Assistant Messages"}
              </p>
            </div>

            <SlackSetupGuidance
              hasSlackConfig={true}
              isPublished={isPublished}
              webhookVerified={chWebhookVerified}
              firstMessageReceived={chFirstMsg}
              webhookUrl={slackWebhookUrl}
              webhookPath={slackWebhookPath}
              isLocalhost={isLocalhost}
              onCreateSlackApp={handleCreateSlackApp}
              creatingSlackApp={creatingSlackApp}
              onConfigure={() => startEditChannel(channel)}
            />
          </>
        ) : (
          <SlackSetupGuidance
            hasSlackConfig={false}
            isPublished={isPublished}
            webhookVerified={chWebhookVerified}
            firstMessageReceived={chFirstMsg}
            webhookUrl={slackWebhookUrl}
            webhookPath={slackWebhookPath}
            isLocalhost={isLocalhost}
            onCreateSlackApp={handleCreateSlackApp}
            creatingSlackApp={creatingSlackApp}
            onConfigure={() => startEditChannel(channel)}
          />
        )}
      </div>
    );
  };

  const renderChannelOverview = (channel: AppChannel) => {
    const summary = getChannelSummary(channel, isPublished);

    if (channel.channel_type === "ag_ui") {
      const config = channel.channel_config as AgUiChannelConfig;
      return (
        <div className="grid gap-4 md:grid-cols-2">
          <ChannelFact label="Access" value={hasToken(config) ? "Token protected" : "No token"} />
          <ChannelFact
            label="Auth"
            value={authLabel(config, hasToken(config) ? "Token" : "Public")}
          />
          <ChannelFact
            label="Thread expiration"
            value={`${(config?.session_expiration_seconds ?? DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS) / 3600}h`}
          />
          <ChannelFact
            label="Rate limit"
            value={
              config?.rate_limit_per_minute
                ? `${config.rate_limit_per_minute}/minute/IP`
                : "Global limit only"
            }
          />
          <ChannelFact
            label="Tool activity"
            value={getAgUiToolVisibilityDisplayName(config?.tool_visibility ?? "generic")}
          />
        </div>
      );
    }

    if (channel.channel_type === "slack") {
      const config = channel.channel_config as SlackChannelConfig;
      const hasCredentials = hasSlackCredentials(config);
      return (
        <div className="grid gap-4 md:grid-cols-2">
          <ChannelFact label="Credentials" value={hasCredentials ? "Configured" : "Missing"} />
          <ChannelFact label="Workspace" value={config?.team_id ?? "Any workspace"} />
          <ChannelFact label="Channel" value={config?.channel_id ?? "Any channel"} />
          <ChannelFact
            label="Session strategy"
            value={getSessionStrategyDisplayName(config?.session_strategy ?? "per_thread")}
          />
          <ChannelFact
            label="Reply mode"
            value={getSlackReplyModeDisplayName(config?.reply_mode ?? "all_messages")}
          />
          <ChannelFact
            label="Health"
            value={`${summary.health}${channel.enabled ? "" : " (disabled)"}`}
          />
        </div>
      );
    }

    if (channel.channel_type === "schedule") {
      const config = channel.channel_config as ScheduleChannelConfig;
      return (
        <div className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2">
            <ChannelFact label="Cron" value={config?.cron_expression ?? "Not set"} />
            <ChannelFact label="Timezone" value={config?.timezone ?? "UTC"} />
            <ChannelFact
              label="Session mode"
              value={getInvocationSessionModeDisplayName(config?.session_mode ?? "shared_session")}
            />
            <ChannelFact label="Health" value={summary.health} />
          </div>
          <ChannelMessagePreview message={config?.message ?? ""} />
        </div>
      );
    }

    if (channel.channel_type === "webhook") {
      const config = channel.channel_config as WebhookChannelConfig;
      return (
        <div className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2">
            <ChannelFact
              label="Authentication"
              value={authLabel(config, hasToken(config) ? "Token" : "Missing")}
            />
            <ChannelFact
              label="Session mode"
              value={getInvocationSessionModeDisplayName(config?.session_mode ?? "shared_session")}
            />
            <ChannelFact label="Health" value={summary.health} />
          </div>
          <ChannelMessagePreview message={config?.message ?? ""} />
        </div>
      );
    }

    if (channel.channel_type === "fcp") {
      const config = channel.channel_config as FcpChannelConfig;
      const tokenConfigured = !!config?.token || !!config?.token_configured;
      const expSeconds =
        typeof config?.session_expiration_seconds === "number"
          ? config.session_expiration_seconds
          : 6 * 60 * 60;
      return (
        <div className="grid gap-4 md:grid-cols-2">
          <ChannelFact
            label="Access"
            value={
              tokenConfigured
                ? "Token protected"
                : config?.anonymous === false
                  ? "Restricted (no token)"
                  : "Anonymous"
            }
          />
          <ChannelFact
            label="Session expiration"
            value={expSeconds === 0 ? "Never" : `${expSeconds / 3600}h`}
          />
          <ChannelFact
            label="Rate limit"
            value={
              config?.rate_limit_per_minute
                ? `${config.rate_limit_per_minute}/minute/IP`
                : "Global limit only"
            }
          />
          <ChannelFact
            label="Response timeout"
            value={`${config?.response_timeout_seconds ?? 120}s`}
          />
          <ChannelFact
            label="Handshake"
            value={config?.handshake?.trim() ? "Custom" : "Auto-generated"}
          />
          <ChannelFact label="Health" value={summary.health} />
        </div>
      );
    }

    const config = channel.channel_config as A2aChannelConfig;
    return (
      <div className="space-y-4">
        <div className="grid gap-4 md:grid-cols-2">
          <ChannelFact
            label="Auth"
            value={authLabel(config, config?.api_key_prefix ? "API key" : "Missing")}
          />
          <ChannelFact
            label="Session mode"
            value={getInvocationSessionModeDisplayName(config?.session_mode ?? "shared_session")}
          />
          <ChannelFact label="Agent card" value={config?.agent_card_name ?? app.name} />
          <ChannelFact label="Health" value={summary.health} />
        </div>
        <ChannelMessagePreview message={config?.message ?? ""} />
      </div>
    );
  };

  const renderChannelEndpoint = (channel: AppChannel) => {
    const endpoint =
      channel.channel_type === "slack"
        ? slackWebhookPath
        : channel.channel_type === "ag_ui"
          ? `/api/v1/apps/${appId}/ag-ui`
          : channel.channel_type === "webhook"
            ? `/api/v1/apps/${appId}/webhooks/${channel.id}`
            : channel.channel_type === "a2a"
              ? `/api/v1/apps/${appId}/a2a/${channel.id}`
              : channel.channel_type === "fcp"
                ? `/api/v1/apps/${appId}/fcp`
                : "";

    if (!endpoint) {
      return null;
    }

    return (
      <div>
        <p className="text-sm font-medium">Endpoint</p>
        <div className="mt-2 flex items-center gap-2 bg-muted p-3">
          <Globe className="h-4 w-4 shrink-0 text-muted-foreground" />
          <code className="flex-1 truncate text-sm">{endpoint}</code>
          <CopyButton value={endpoint} />
        </div>
      </div>
    );
  };

  const renderChannelDetail = (channel: AppChannel) => {
    const summary = getChannelSummary(channel, isPublished);
    const isEditingThisChannel = editingChannelId === channel.id;

    return (
      <Card className="min-h-[520px]">
        <CardHeader className="space-y-4">
          <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
            <div className="space-y-2">
              <div className="flex flex-wrap items-center gap-2">
                <CardTitle>{getChannelTypeDisplayName(channel.channel_type)}</CardTitle>
                <Badge variant={channel.enabled ? "default" : "secondary"}>
                  {channel.enabled ? "Enabled" : "Disabled"}
                </Badge>
                <Badge variant={summary.healthVariant}>{summary.health}</Badge>
              </div>
              <p className="text-sm text-muted-foreground">{summary.target}</p>
            </div>
            {!isReadOnly && (
              <div className="flex flex-wrap gap-2">
                {!isEditingThisChannel && (
                  <Button size="sm" variant="outline" onClick={() => startEditChannel(channel)}>
                    <Pencil className="h-3 w-3" />
                    Configure
                  </Button>
                )}
                {channel.channel_type === "a2a" && !isEditingThisChannel && (
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => rotateA2aMutation.mutate(channel.id)}
                    disabled={rotateA2aMutation.isPending && rotatingA2aChannelId === channel.id}
                  >
                    <RefreshCw className="h-3 w-3" />
                    {rotateA2aMutation.isPending && rotatingA2aChannelId === channel.id
                      ? "Rotating"
                      : "Rotate key"}
                  </Button>
                )}
                {(app.channels ?? []).length > 1 && !isEditingThisChannel && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => deleteChannelMutation.mutate(channel.id)}
                    disabled={deleteChannelMutation.isPending}
                  >
                    <Trash2 className="h-3 w-3" />
                    Delete
                  </Button>
                )}
              </div>
            )}
          </div>
        </CardHeader>
        <CardContent>
          {isEditingThisChannel ? (
            renderChannelForm(updateChannelMutation.isPending, channel.id)
          ) : (
            <Tabs value={channelDetailTab} onValueChange={setChannelDetailTab}>
              <TabsList className="mb-4">
                <TabsTrigger value="overview">Overview</TabsTrigger>
                <TabsTrigger value="setup">Setup</TabsTrigger>
                <TabsTrigger value="settings">Settings</TabsTrigger>
              </TabsList>
              <TabsContent value="overview" className="space-y-5">
                {renderChannelEndpoint(channel)}
                {renderChannelOverview(channel)}
              </TabsContent>
              <TabsContent value="setup">{renderChannelDisplay(channel)}</TabsContent>
              <TabsContent value="settings" className="space-y-5">
                <div className="rounded-md border p-3">
                  <div className="flex items-center justify-between gap-3">
                    <div>
                      <p className="text-sm font-medium">Channel status</p>
                      <p className="text-sm text-muted-foreground">
                        Disabled channels remain configured but stop accepting or emitting new
                        invocations. Use Edit settings to change this channel state.
                      </p>
                    </div>
                    <Badge variant={channel.enabled ? "default" : "secondary"}>
                      {channel.enabled ? "Enabled" : "Disabled"}
                    </Badge>
                  </div>
                </div>
                {renderChannelOverview(channel)}
                {!isReadOnly && (
                  <Button size="sm" variant="outline" onClick={() => startEditChannel(channel)}>
                    <Pencil className="h-3 w-3" />
                    Edit settings
                  </Button>
                )}
              </TabsContent>
            </Tabs>
          )}
        </CardContent>
      </Card>
    );
  };

  const channels = app.channels ?? [];
  const selectedChannel =
    channels.find((channel) => channel.id === selectedChannelId) ?? channels[0] ?? null;

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/apps"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Apps
      </Link>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            <Rocket className="w-6 h-6" />
            <span className={getEntityNameClassName(app.status)}>{app.name}</span>
            <CopyButton value={app.id} />
            <Badge variant={getEntityStatusBadgeVariant(app.status)}>{app.status}</Badge>
          </h1>
          {app.description && <p className="text-muted-foreground mt-1">{app.description}</p>}
        </div>
        <div className="flex gap-2">
          {isPublished ? (
            <Button
              variant="outline"
              onClick={() => unpublishApp.mutate(app.id)}
              disabled={unpublishApp.isPending || isReadOnly}
            >
              <GlobeLock className="w-4 h-4 mr-2" />
              {unpublishApp.isPending ? "Unpublishing..." : "Unpublish"}
            </Button>
          ) : (
            <Button
              variant="default"
              onClick={() => publishApp.mutate(app.id)}
              disabled={publishApp.isPending || !canPublishApp || isReadOnly}
            >
              <Globe className="w-4 h-4 mr-2" />
              {publishApp.isPending ? "Publishing..." : "Publish"}
            </Button>
          )}
          {!isArchived && (
            <Button
              variant="outline"
              onClick={handleArchive}
              disabled={deleteAppMutation.isPending || isReadOnly}
            >
              {deleteAppMutation.isPending ? "Archiving..." : "Archive"}
            </Button>
          )}
          {isArchived && canDangerousDelete && (
            <Button
              variant="destructive"
              className="text-destructive hover:text-destructive"
              onClick={() => setShowDeleteDialog(true)}
            >
              <Trash2 className="w-4 h-4 mr-2" />
              Delete
            </Button>
          )}
        </div>
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
          <div className="grid gap-6 xl:grid-cols-[minmax(280px,340px),1fr]">
            <Card className="h-fit">
              <CardHeader className="flex flex-row items-center justify-between">
                <CardTitle>Channels</CardTitle>
                {!isReadOnly && !showAddChannel && (
                  <Button variant="outline" size="sm" onClick={startAddChannel}>
                    <Plus className="h-3 w-3" />
                    Add
                  </Button>
                )}
              </CardHeader>
              <CardContent className="space-y-3">
                {channels.length === 0 && (
                  <div className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
                    No channels configured.
                  </div>
                )}

                {channels.map((channel) => {
                  const summary = getChannelSummary(channel, isPublished);
                  const selected = selectedChannel?.id === channel.id && !showAddChannel;

                  return (
                    <button
                      key={channel.id}
                      type="button"
                      onClick={() => {
                        setSelectedChannelId(channel.id);
                        setShowAddChannel(false);
                      }}
                      aria-pressed={selected}
                      aria-label={`Select channel ${summary.target}`}
                      className={cn(
                        "w-full rounded-md border border-border p-3 text-left transition-colors outline-none hover:bg-muted/50 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50",
                        selected && "bg-muted",
                      )}
                    >
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0 space-y-1">
                          <div className="flex flex-wrap items-center gap-1.5">
                            <Badge variant="outline">
                              {getChannelTypeDisplayName(channel.channel_type)}
                            </Badge>
                            {!channel.enabled && <Badge variant="secondary">Disabled</Badge>}
                          </div>
                          <p className="truncate text-sm font-medium">{summary.target}</p>
                          <p className="text-xs text-muted-foreground">{summary.group}</p>
                        </div>
                        <Badge variant={summary.healthVariant}>{summary.health}</Badge>
                      </div>
                    </button>
                  );
                })}
              </CardContent>
            </Card>

            {showAddChannel ? (
              <Card className="min-h-[520px]">
                <CardHeader>
                  <CardTitle>Add Channel</CardTitle>
                </CardHeader>
                <CardContent>
                  {renderChannelForm(
                    addChannelMutation.isPending || addA2aMutation.isPending,
                    "new",
                  )}
                </CardContent>
              </Card>
            ) : selectedChannel ? (
              renderChannelDetail(selectedChannel)
            ) : (
              <Card className="min-h-[320px]">
                <CardContent className="flex h-full min-h-[320px] items-center justify-center text-sm text-muted-foreground">
                  Add a channel to expose this app.
                </CardContent>
              </Card>
            )}
          </div>

          {appBudgetsEnabled && <AppBudgetsCard app={app} readOnly={isReadOnly} />}
        </div>

        {/* Sidebar */}
        <div className="space-y-6">
          <Card>
            <CardHeader className="flex flex-row items-center justify-between">
              <CardTitle>Configuration</CardTitle>
              {!editingBasic && !isReadOnly && (
                <Button variant="outline" size="sm" onClick={startEditBasic}>
                  <Pencil className="w-3 h-3 mr-1" />
                  Edit
                </Button>
              )}
            </CardHeader>
            <CardContent>
              {editingBasic ? (
                <div className="space-y-4">
                  <div>
                    <Label htmlFor="edit_name">Name</Label>
                    <Input
                      id="edit_name"
                      value={editName}
                      onChange={(e) => setEditName(e.target.value)}
                      required
                    />
                  </div>

                  <div>
                    <Label htmlFor="edit_description">Description</Label>
                    <Input
                      id="edit_description"
                      value={editDescription}
                      onChange={(e) => setEditDescription(e.target.value)}
                    />
                  </div>

                  <div>
                    <Label>Harness</Label>
                    <HarnessSelect value={editHarnessId} onValueChange={setEditHarnessId} />
                  </div>

                  <div>
                    <Label>Agent</Label>
                    <AgentSelect
                      value={editAgentId}
                      onValueChange={(value) => {
                        setEditAgentId(value);
                        setEditAgentVersionId("");
                        if (!value) setEditAgentVersionPolicy("default");
                      }}
                      includeNoneOption
                    />
                  </div>

                  {agentVersionsEnabled && editAgentId && (
                    <div className="space-y-3 border p-3">
                      <div>
                        <Label>Agent version</Label>
                        <Select
                          value={editAgentVersionPolicy}
                          onValueChange={(value) => {
                            const policy = value as AgentVersionPolicy;
                            setEditAgentVersionPolicy(policy);
                            if (policy !== "pinned") setEditAgentVersionId("");
                          }}
                        >
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="default">Default version</SelectItem>
                            <SelectItem value="latest">Latest version</SelectItem>
                            <SelectItem value="pinned">Pinned version</SelectItem>
                          </SelectContent>
                        </Select>
                      </div>
                      {editAgentVersionPolicy === "pinned" && (
                        <div>
                          <Label>Pinned version</Label>
                          <Select value={editAgentVersionId} onValueChange={setEditAgentVersionId}>
                            <SelectTrigger>
                              <SelectValue placeholder="Select version" />
                            </SelectTrigger>
                            <SelectContent>
                              {publishedEditableAgentVersions.map((version) => (
                                <SelectItem key={version.id} value={version.id}>
                                  {version.version}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </div>
                      )}
                    </div>
                  )}

                  <div>
                    <Label>Agent identity</Label>
                    <AgentIdentitySelect
                      value={editAgentIdentityId}
                      onValueChange={setEditAgentIdentityId}
                    />
                  </div>

                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      onClick={saveBasic}
                      disabled={updateApp.isPending || !editName || !editHarnessId}
                    >
                      <Check className="w-3 h-3 mr-1" />
                      {updateApp.isPending ? "Saving..." : "Save"}
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => setEditingBasic(false)}>
                      <X className="w-3 h-3 mr-1" />
                      Cancel
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="space-y-4">
                  <div>
                    <p className="text-sm font-medium">Harness</p>
                    <p
                      className={`text-sm ${getEntityReferenceClassName(harness?.status ?? "deleted")}`}
                    >
                      {getEntityReferenceLabel({
                        kind: "Harness",
                        name: getDisplayName(harness),
                        status: harness?.status ?? "deleted",
                      })}
                    </p>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Agent</p>
                    <p
                      className={`text-sm ${getEntityReferenceClassName(
                        app.agent_id ? (agent?.status ?? "deleted") : undefined,
                      )}`}
                    >
                      {app.agent_id
                        ? getEntityReferenceLabel({
                            kind: "Agent",
                            name: getDisplayName(agent),
                            status: agent?.status ?? "deleted",
                          })
                        : "None assigned"}
                    </p>
                    {agentVersionsEnabled && app.agent_id && (
                      <p className="mt-1 text-xs text-muted-foreground">
                        Version policy: {app.agent_version_policy}
                        {app.agent_version_policy === "pinned" && app.agent_version_id
                          ? ` · ${pinnedVersion?.version ?? app.agent_version_id}`
                          : ""}
                      </p>
                    )}
                  </div>

                  <div>
                    <p className="text-sm font-medium">Channels</p>
                    <div className="flex gap-1 flex-wrap">
                      {(app.channels ?? []).map((ch: AppChannel) => (
                        <Badge key={ch.id} variant="outline">
                          {getChannelTypeDisplayName(ch.channel_type)}
                        </Badge>
                      ))}
                      {(app.channels ?? []).length === 0 && (
                        <span className="text-sm text-muted-foreground">None configured</span>
                      )}
                    </div>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Created</p>
                    <p className="text-sm text-muted-foreground">
                      {new Date(app.created_at).toLocaleString()}
                    </p>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Updated</p>
                    <p className="text-sm text-muted-foreground">
                      {new Date(app.updated_at).toLocaleString()}
                    </p>
                  </div>

                  {app.published_at && (
                    <div>
                      <p className="text-sm font-medium">Published</p>
                      <p className="text-sm text-muted-foreground">
                        {new Date(app.published_at).toLocaleString()}
                      </p>
                    </div>
                  )}
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      </div>

      {/* A2A revealed-key dialog: shown exactly once per create/rotate */}
      <Dialog
        open={!!a2aRevealedKey}
        onOpenChange={(open) => {
          if (!open) setA2aRevealedKey(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{a2aRevealedKey?.title ?? "A2A API key"}</DialogTitle>
            <DialogDescription>
              Copy this API key now. It will not be shown again — only the SHA-256 hash is stored.
              If you lose it, rotate the key from the channel card.
            </DialogDescription>
          </DialogHeader>
          {a2aRevealedKey && (
            <div className="flex items-center gap-2 bg-muted p-3">
              <code className="flex-1 break-all font-mono text-xs">{a2aRevealedKey.apiKey}</code>
              <CopyButton value={a2aRevealedKey.apiKey} />
            </div>
          )}
          <DialogFooter>
            <Button onClick={() => setA2aRevealedKey(null)}>I&apos;ve saved it</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete confirmation dialog */}
      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete App</DialogTitle>
            <DialogDescription>
              Permanently delete the archived app &quot;{app.name}&quot;? Existing references will
              render as deleted tombstones.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowDeleteDialog(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={handleDelete}
              disabled={destroyAppMutation.isPending}
            >
              {destroyAppMutation.isPending ? "Deleting..." : "Delete"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

export default function AppDetailPage({ params }: { params: Promise<{ appId: string }> }) {
  const detailV2Enabled = useFeatureFlag("apps.detailV2");
  const { appId } = use(params);

  if (detailV2Enabled) {
    return <AppDetailV2 appId={appId} />;
  }

  return <AppDetailPageLegacy params={Promise.resolve({ appId })} />;
}
