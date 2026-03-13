"use client";

import { use, useEffect, useMemo, useRef, useState } from "react";
import { usePathname } from "next/navigation";
import Link from "next/link";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Input } from "@/components/ui/input";
import {
  ArrowLeft,
  Sparkles,
  MessageSquare,
  Folder,
  Activity,
  Zap,
  Database,
  Bot,
  GitBranch,
  Pencil,
  Check,
  X,
  CalendarClock,
  Wrench,
} from "lucide-react";
import { cn, shortenId } from "@/lib/utils";
import { CopyButton } from "@/components/ui/copy-button";
import { useUpdateSession } from "@/hooks/use-sessions";
import { SessionProvider, useSessionContext } from "./session-context";

// Helper function to format token counts in a compact way
function formatTokens(tokens: number): string {
  if (tokens >= 1000000) {
    return `${(tokens / 1000000).toFixed(1)}M`;
  }
  if (tokens >= 1000) {
    return `${(tokens / 1000).toFixed(1)}K`;
  }
  return tokens.toString();
}

interface SessionLayoutProps {
  children: React.ReactNode;
  params: Promise<{ sessionId: string }>;
}

export default function SessionLayout({ children, params }: SessionLayoutProps) {
  const { sessionId } = use(params);

  return (
    <SessionProvider sessionId={sessionId}>
      <SessionLayoutContent sessionId={sessionId}>{children}</SessionLayoutContent>
    </SessionProvider>
  );
}

interface SessionLayoutContentProps {
  children: React.ReactNode;
  sessionId: string;
}

function EditableSessionTitle({
  sessionId,
  title,
  fallback,
}: {
  sessionId: string;
  title: string | null;
  fallback: string;
}) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(title ?? "");
  const inputRef = useRef<HTMLInputElement>(null);
  const updateSession = useUpdateSession();

  useEffect(() => {
    if (editing) {
      setValue(title ?? "");
      // Wait for next frame so the input is mounted
      requestAnimationFrame(() => inputRef.current?.select());
    }
  }, [editing, title]);

  function save() {
    const trimmed = value.trim();
    // Skip if unchanged
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
          onChange={(e: React.ChangeEvent<HTMLInputElement>) => setValue(e.target.value)}
          onKeyDown={(e: React.KeyboardEvent) => {
            if (e.key === "Enter") save();
            if (e.key === "Escape") setEditing(false);
          }}
          className="h-8 text-xl font-bold w-64"
          placeholder={fallback}
        />
        <button
          onClick={save}
          className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground"
          aria-label="Save title"
        >
          <Check className="w-4 h-4" />
        </button>
        <button
          onClick={() => setEditing(false)}
          className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground"
          aria-label="Cancel editing"
        >
          <X className="w-4 h-4" />
        </button>
      </div>
    );
  }

  return (
    <h1 className="text-xl font-bold flex items-center gap-2">
      {title || fallback}
      <button
        onClick={() => setEditing(true)}
        className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground opacity-0 group-hover/title:opacity-100 transition-opacity"
        aria-label="Edit session title"
      >
        <Pencil className="w-3.5 h-3.5" />
      </button>
      <CopyButton value={sessionId} />
    </h1>
  );
}

function SessionLayoutContent({ children, sessionId }: SessionLayoutContentProps) {
  const pathname = usePathname();
  const { agent, session, llmModel, sessionLoading, effectiveStatus, liveUsage, agentId } =
    useSessionContext();

  // Determine active tab from pathname
  const getActiveTab = () => {
    if (pathname.endsWith("/files")) return "files";
    if (pathname.endsWith("/trajectory")) return "trajectory";
    if (pathname.endsWith("/events")) return "events";
    if (pathname.endsWith("/storage")) return "storage";
    if (pathname.endsWith("/resources")) return "resources";
    if (pathname.endsWith("/schedules")) return "schedules";
    return "chat"; // Default to chat (includes /chat and base path)
  };
  const activeTab = getActiveTab();

  const basePath = `/sessions/${sessionId}`;
  const getTabClassName = (isActive: boolean) =>
    cn(
      "inline-flex h-9 items-center gap-2 border px-3 text-sm transition-colors",
      isActive
        ? "border-primary bg-card text-foreground"
        : "border-transparent bg-transparent text-muted-foreground hover:border-border hover:bg-card hover:text-foreground",
    );

  // Compute active feature set from session capabilities
  const features = useMemo(() => new Set(session?.features ?? []), [session?.features]);
  const hasFeature = (feature: string) => features.has(feature);

  if (sessionLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!session) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-destructive">Session not found</div>
        <Link href="/sessions" className="text-sm text-muted-foreground hover:text-foreground">
          Back to sessions
        </Link>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="border-b bg-background/90 p-4">
        <Link
          href="/sessions"
          className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-2"
        >
          <ArrowLeft className="icon-sharp mr-2 h-4 w-4" />
          Back to Sessions
        </Link>

        <div className="flex items-center justify-between">
          <div className="group/title">
            <EditableSessionTitle
              sessionId={sessionId}
              title={session.title}
              fallback={`Session ${shortenId(session.id)}`}
            />
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              {agent && (
                <Link
                  href={`/agents/${agentId}`}
                  className="inline-flex items-center gap-1 hover:text-foreground"
                >
                  <Bot className="icon-sharp h-3 w-3" />
                  {agent.name}
                </Link>
              )}
              <span>•</span>
              <span>
                {activeTab === "files"
                  ? "Workspace: /workspace"
                  : `Started ${new Date(session.created_at).toLocaleString()}`}
              </span>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {liveUsage && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger>
                    <Badge variant="outline" className="gap-1 cursor-help">
                      <Zap className="icon-sharp h-3 w-3" />
                      {formatTokens(liveUsage.input_tokens)} /{" "}
                      {formatTokens(liveUsage.output_tokens)}
                    </Badge>
                  </TooltipTrigger>
                  <TooltipContent>
                    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
                      <dt className="text-muted-foreground">Input</dt>
                      <dd>{liveUsage.input_tokens.toLocaleString()}</dd>
                      <dt className="text-muted-foreground">Output</dt>
                      <dd>{liveUsage.output_tokens.toLocaleString()}</dd>
                      {(liveUsage.cache_read_tokens ?? 0) > 0 && (
                        <>
                          <dt className="text-muted-foreground">Cache read</dt>
                          <dd>{liveUsage.cache_read_tokens!.toLocaleString()}</dd>
                        </>
                      )}
                      {(liveUsage.cache_creation_tokens ?? 0) > 0 && (
                        <>
                          <dt className="text-muted-foreground">Cache created</dt>
                          <dd>{liveUsage.cache_creation_tokens!.toLocaleString()}</dd>
                        </>
                      )}
                    </dl>
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )}
            {llmModel && (
              <Badge variant="outline" className="gap-1">
                <Sparkles className="icon-sharp h-3 w-3" />
                {llmModel.display_name}
              </Badge>
            )}
            {effectiveStatus === "active" && <Badge variant="default">Processing...</Badge>}
            {effectiveStatus === "idle" && <Badge variant="secondary">Ready</Badge>}
            {effectiveStatus === "started" && <Badge variant="outline">New</Badge>}
          </div>
        </div>

        {/* Tabs */}
        <div className="flex gap-1 mt-4">
          <Link href={`${basePath}/chat`} className={getTabClassName(activeTab === "chat")}>
            <MessageSquare className="icon-sharp h-4 w-4" />
            Chat
          </Link>
          {hasFeature("file_system") && (
            <Link href={`${basePath}/files`} className={getTabClassName(activeTab === "files")}>
              <Folder className="icon-sharp h-4 w-4" />
              Workspace
            </Link>
          )}
          {(hasFeature("secrets") || hasFeature("key_value")) && (
            <Link href={`${basePath}/storage`} className={getTabClassName(activeTab === "storage")}>
              <Database className="icon-sharp h-4 w-4" />
              Storage
            </Link>
          )}
          {hasFeature("leased_resources") && (
            <Link
              href={`${basePath}/resources`}
              className={getTabClassName(activeTab === "resources")}
            >
              <Wrench className="icon-sharp h-4 w-4" />
              Resources
            </Link>
          )}
          {hasFeature("schedules") && (
            <Link
              href={`${basePath}/schedules`}
              className={getTabClassName(activeTab === "schedules")}
            >
              <CalendarClock className="icon-sharp h-4 w-4" />
              Schedules
              {(session.active_schedule_count ?? 0) > 0 && (
                <Badge variant="secondary" className="h-5 min-w-5 px-1 text-xs">
                  {session.active_schedule_count}
                </Badge>
              )}
            </Link>
          )}
          <Link href={`${basePath}/events`} className={getTabClassName(activeTab === "events")}>
            <Activity className="icon-sharp h-4 w-4" />
            Events
          </Link>
          <Link
            href={`${basePath}/trajectory`}
            className={getTabClassName(activeTab === "trajectory")}
          >
            <GitBranch className="icon-sharp h-4 w-4" />
            Trajectory
          </Link>
        </div>
      </div>

      {/* Tab content */}
      {children}
    </div>
  );
}
