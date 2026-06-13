"use client";

import {
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type ComponentType,
  type KeyboardEvent,
} from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import type { Agent, ModelWithProvider, Session, SessionStatus, TokenUsage } from "@/lib/api/types";
import { buttonVariants } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { CopyButton } from "@/components/ui/copy-button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { useUpdateSession } from "@/hooks/use-sessions";
import {
  getDisplayName,
  getEntityReferenceClassName,
  getEntityReferenceLabel,
} from "@/lib/entity-lifecycle";
import { formatTokens } from "@/lib/formatting";
import { cn, shortenId } from "@/lib/utils";
import {
  ArrowLeft,
  Bot,
  CalendarClock,
  Check,
  ChevronDown,
  Database,
  Folder,
  GitBranch,
  Activity,
  ChartNoAxesColumn,
  MessageSquare,
  Pencil,
  Sparkles,
  Wrench,
  X,
  Zap,
} from "lucide-react";

export type SessionNavKey =
  | "chat"
  | "files"
  | "storage"
  | "resources"
  | "schedules"
  | "context"
  | "events"
  | "trajectory";

export interface SessionNavItem {
  key: SessionNavKey;
  label: string;
  href: string;
  icon: ComponentType<{ className?: string }>;
  badge?: string;
}

function EditableSessionTitle({
  sessionId,
  title,
  fallback,
  editable,
}: {
  sessionId: string;
  title: string | null;
  fallback: string;
  editable: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(title ?? "");
  const inputRef = useRef<HTMLInputElement>(null);
  const updateSession = useUpdateSession();

  useEffect(() => {
    if (editing) {
      setValue(title ?? "");
      requestAnimationFrame(() => inputRef.current?.select());
    }
  }, [editing, title]);

  function save() {
    const trimmed = value.trim();
    if (trimmed === (title ?? "")) {
      setEditing(false);
      return;
    }
    updateSession.mutate(
      { sessionId, request: { title: trimmed || undefined } },
      { onSettled: () => setEditing(false) },
    );
  }

  if (editing) {
    return (
      <div className="flex items-center gap-1">
        <Input
          ref={inputRef}
          value={value}
          onChange={(e: ChangeEvent<HTMLInputElement>) => setValue(e.target.value)}
          onKeyDown={(e: KeyboardEvent) => {
            if (e.key === "Enter") save();
            if (e.key === "Escape") setEditing(false);
          }}
          className="h-8 w-64 text-xl font-bold"
          placeholder={fallback}
        />
        <button
          onClick={save}
          className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          aria-label="Save title"
        >
          <Check className="h-4 w-4" />
        </button>
        <button
          onClick={() => setEditing(false)}
          className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          aria-label="Cancel editing"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    );
  }

  return (
    <h1 className="flex items-center gap-2 text-xl font-bold">
      {title || fallback}
      {editable && (
        <button
          onClick={() => setEditing(true)}
          className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          aria-label="Edit session title"
        >
          <Pencil className="h-3.5 w-3.5" />
        </button>
      )}
      <CopyButton value={sessionId} />
    </h1>
  );
}

export function SessionUsageBadge({ usage }: { usage: TokenUsage }) {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger>
          <Badge variant="outline" className="cursor-help gap-1">
            <Zap className="icon-sharp h-3 w-3" />
            {formatTokens(usage.input_tokens)} / {formatTokens(usage.output_tokens)}
          </Badge>
        </TooltipTrigger>
        <TooltipContent>
          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
            <dt className="text-muted-foreground">Input</dt>
            <dd>{usage.input_tokens.toLocaleString()}</dd>
            <dt className="text-muted-foreground">Output</dt>
            <dd>{usage.output_tokens.toLocaleString()}</dd>
            {(usage.cache_read_tokens ?? 0) > 0 && (
              <>
                <dt className="text-muted-foreground">Cache read</dt>
                <dd>{usage.cache_read_tokens!.toLocaleString()}</dd>
              </>
            )}
            {(usage.cache_creation_tokens ?? 0) > 0 && (
              <>
                <dt className="text-muted-foreground">Cache created</dt>
                <dd>{usage.cache_creation_tokens!.toLocaleString()}</dd>
              </>
            )}
          </dl>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

export function SessionStatusBadge({ status }: { status: SessionStatus | undefined }) {
  if (status === "active") {
    return <Badge variant="default">Processing...</Badge>;
  }

  if (status === "idle") {
    return <Badge variant="secondary">Ready</Badge>;
  }

  if (status === "waiting_for_tool_results") {
    return <Badge variant="outline">Waiting on tools</Badge>;
  }

  if (status === "started") {
    return <Badge variant="outline">New</Badge>;
  }

  return null;
}

export function buildSessionNavigation({
  basePath,
  features,
  activeScheduleCount,
}: {
  basePath: string;
  features: Set<string>;
  activeScheduleCount?: number;
}): SessionNavItem[] {
  const hasFeature = (feature: string) => features.has(feature);

  return [
    {
      key: "chat",
      label: "Chat",
      href: `${basePath}/chat`,
      icon: MessageSquare,
    },
    ...(hasFeature("file_system")
      ? [
          {
            key: "files" as const,
            label: "Workspace",
            href: `${basePath}/files`,
            icon: Folder,
          },
        ]
      : []),
    ...(hasFeature("secrets") || hasFeature("key_value")
      ? [
          {
            key: "storage" as const,
            label: "Storage",
            href: `${basePath}/storage`,
            icon: Database,
          },
        ]
      : []),
    ...(hasFeature("leased_resources")
      ? [
          {
            key: "resources" as const,
            // Route stays /resources — renaming it would break existing links.
            label: "Tasks",
            href: `${basePath}/resources`,
            icon: Wrench,
          },
        ]
      : []),
    ...(hasFeature("schedules")
      ? [
          {
            key: "schedules" as const,
            label: "Schedules",
            href: `${basePath}/schedules`,
            icon: CalendarClock,
            badge:
              activeScheduleCount && activeScheduleCount > 0
                ? String(activeScheduleCount)
                : undefined,
          },
        ]
      : []),
    {
      key: "context",
      label: "Context",
      href: `${basePath}/context`,
      icon: ChartNoAxesColumn,
    },
    {
      key: "events",
      label: "Events",
      href: `${basePath}/events`,
      icon: Activity,
    },
    {
      key: "trajectory",
      label: "Trajectory",
      href: `${basePath}/trajectory`,
      icon: GitBranch,
    },
  ];
}

export function SessionHeader({
  sessionId,
  session,
  agent,
  agentId,
  llmModel,
  effectiveStatus,
  liveUsage,
  activeTab,
  navigationItems,
  secondaryMetaText,
  backHref = "/sessions",
  backLabel = "Back to Sessions",
  allowTitleEditing = true,
}: {
  sessionId: string;
  session: Session;
  agent?: Agent;
  agentId?: string;
  llmModel?: ModelWithProvider;
  effectiveStatus?: SessionStatus;
  liveUsage?: TokenUsage;
  activeTab: SessionNavKey;
  navigationItems: SessionNavItem[];
  secondaryMetaText: string;
  backHref?: string;
  backLabel?: string;
  allowTitleEditing?: boolean;
}) {
  const router = useRouter();
  const agentReferenceLabel =
    session.agent_id != null
      ? getEntityReferenceLabel({
          kind: "Agent",
          name: getDisplayName(agent),
          status: agent?.status ?? "deleted",
        })
      : null;
  const agentReferenceStatus = session.agent_id != null ? (agent?.status ?? "deleted") : null;
  const getNavigationClassName = (isActive: boolean) =>
    cn(
      buttonVariants({ variant: "outline", size: "sm" }),
      isActive ? "border-primary bg-card text-foreground" : "text-muted-foreground",
    );
  const chatItem = navigationItems[0];
  const advancedNavigationItems = navigationItems.slice(1);
  const activeAdvancedItem = advancedNavigationItems.find((item) => item.key === activeTab);

  return (
    <div className="border-b border-border/70 bg-background/80 px-4 py-3 backdrop-blur-[1px]">
      <Link
        href={backHref}
        className="mb-1.5 inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="icon-sharp mr-2 h-4 w-4" />
        {backLabel}
      </Link>

      <div className="flex items-center justify-between gap-4">
        <div className="group/title min-w-0">
          <EditableSessionTitle
            sessionId={sessionId}
            title={session.title}
            fallback={`Session ${shortenId(session.id)}`}
            editable={allowTitleEditing}
          />
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            {agentReferenceLabel &&
              (agent && agentId ? (
                <Link
                  href={`/agents/${agentId}`}
                  className="inline-flex items-center gap-1 hover:text-foreground"
                >
                  <Bot className="icon-sharp h-3 w-3" />
                  <span className={getEntityReferenceClassName(agentReferenceStatus)}>
                    {agentReferenceLabel}
                  </span>
                </Link>
              ) : (
                <span className="inline-flex items-center gap-1">
                  <Bot className="icon-sharp h-3 w-3" />
                  <span className={getEntityReferenceClassName(agentReferenceStatus)}>
                    {agentReferenceLabel}
                  </span>
                </span>
              ))}
            <span>•</span>
            <span>{secondaryMetaText}</span>
          </div>
        </div>

        <div className="flex flex-wrap items-center justify-end gap-2">
          {liveUsage && <SessionUsageBadge usage={liveUsage} />}

          {llmModel && (
            <Badge variant="outline" className="gap-1">
              <Sparkles className="icon-sharp h-3 w-3" />
              {llmModel.display_name}
            </Badge>
          )}

          <SessionStatusBadge status={effectiveStatus} />

          {chatItem && (
            <Link
              href={chatItem.href}
              className={getNavigationClassName(activeTab === chatItem.key)}
              aria-current={activeTab === chatItem.key ? "page" : undefined}
            >
              <chatItem.icon className="icon-sharp h-4 w-4" />
              {chatItem.label}
            </Link>
          )}

          {advancedNavigationItems.length > 0 && (
            <DropdownMenu>
              <DropdownMenuTrigger
                className={getNavigationClassName(activeTab !== "chat")}
                aria-label="Advanced navigation"
              >
                Advanced
                {activeAdvancedItem && (
                  <span className="text-xs text-muted-foreground">{activeAdvancedItem.label}</span>
                )}
                <ChevronDown className="icon-sharp h-4 w-4 opacity-60" />
              </DropdownMenuTrigger>
              <DropdownMenuPositioner align="end">
                <DropdownMenuContent className="w-56">
                  <DropdownMenuGroup>
                    <DropdownMenuLabel>Advanced Views</DropdownMenuLabel>
                    {advancedNavigationItems.map((item) => (
                      <DropdownMenuItem
                        key={item.key}
                        onClick={() => router.push(item.href)}
                        className="flex items-center justify-between gap-3"
                      >
                        <span className="flex min-w-0 items-center gap-2">
                          <item.icon className="icon-sharp h-4 w-4" />
                          <span>{item.label}</span>
                        </span>
                        <span className="flex items-center gap-2">
                          {item.badge && <DropdownMenuShortcut>{item.badge}</DropdownMenuShortcut>}
                          {activeTab === item.key && (
                            <Check className="icon-sharp h-4 w-4 text-primary" />
                          )}
                        </span>
                      </DropdownMenuItem>
                    ))}
                  </DropdownMenuGroup>
                </DropdownMenuContent>
              </DropdownMenuPositioner>
            </DropdownMenu>
          )}
        </div>
      </div>
    </div>
  );
}
