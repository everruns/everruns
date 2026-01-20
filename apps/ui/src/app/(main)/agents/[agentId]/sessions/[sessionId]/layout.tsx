"use client";

import { use } from "react";
import { usePathname } from "next/navigation";
import Link from "next/link";
import { buttonVariants } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ArrowLeft, Sparkles, MessageSquare, Folder, Activity, Zap, Database } from "lucide-react";
import { cn } from "@/lib/utils";
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
  params: Promise<{ agentId: string; sessionId: string }>;
}

export default function SessionLayout({ children, params }: SessionLayoutProps) {
  const { agentId, sessionId } = use(params);

  return (
    <SessionProvider agentId={agentId} sessionId={sessionId}>
      <SessionLayoutContent agentId={agentId} sessionId={sessionId}>
        {children}
      </SessionLayoutContent>
    </SessionProvider>
  );
}

interface SessionLayoutContentProps {
  children: React.ReactNode;
  agentId: string;
  sessionId: string;
}

function SessionLayoutContent({ children, agentId, sessionId }: SessionLayoutContentProps) {
  const pathname = usePathname();
  const { agent, session, llmModel, sessionLoading, effectiveStatus, liveUsage } = useSessionContext();

  // Determine active tab from pathname
  const getActiveTab = () => {
    if (pathname.endsWith("/files")) return "files";
    if (pathname.endsWith("/events")) return "events";
    if (pathname.endsWith("/storage")) return "storage";
    return "chat"; // Default to chat (includes /chat and base path)
  };
  const activeTab = getActiveTab();

  const basePath = `/agents/${agentId}/sessions/${sessionId}`;

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
        <div className="text-red-500">Session not found</div>
        <Link href={`/agents/${agentId}`} className="text-blue-500 hover:underline">
          Back to agent
        </Link>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-[calc(100vh-4rem)]">
      {/* Header */}
      <div className="border-b p-4">
        <Link
          href={`/agents/${agentId}`}
          className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-2"
        >
          <ArrowLeft className="w-4 h-4 mr-2" />
          Back to {agent?.name || "Agent"}
        </Link>

        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-xl font-bold">
              {session.title || `Session ${session.id.slice(0, 8)}`}
            </h1>
            <p className="text-sm text-muted-foreground">
              {activeTab === "files"
                ? "Session file system"
                : `Started ${new Date(session.created_at).toLocaleString()}`}
            </p>
          </div>
          <div className="flex items-center gap-2">
            {liveUsage && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger>
                    <Badge variant="outline" className="gap-1 cursor-help">
                      <Zap className="w-3 h-3" />
                      {formatTokens(liveUsage.input_tokens)} / {formatTokens(liveUsage.output_tokens)}
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
                <Sparkles className="w-3 h-3" />
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
          <Link
            href={`${basePath}/chat`}
            className={cn(
              buttonVariants({
                variant: activeTab === "chat" ? "default" : "ghost",
                size: "sm",
              }),
              "gap-2"
            )}
          >
            <MessageSquare className="h-4 w-4" />
            Chat
          </Link>
          <Link
            href={`${basePath}/files`}
            className={cn(
              buttonVariants({
                variant: activeTab === "files" ? "default" : "ghost",
                size: "sm",
              }),
              "gap-2"
            )}
          >
            <Folder className="h-4 w-4" />
            Files
          </Link>
          <Link
            href={`${basePath}/events`}
            className={cn(
              buttonVariants({
                variant: activeTab === "events" ? "default" : "ghost",
                size: "sm",
              }),
              "gap-2"
            )}
          >
            <Activity className="h-4 w-4" />
            Events
          </Link>
          <Link
            href={`${basePath}/storage`}
            className={cn(
              buttonVariants({
                variant: activeTab === "storage" ? "default" : "ghost",
                size: "sm",
              }),
              "gap-2"
            )}
          >
            <Database className="h-4 w-4" />
            Storage
          </Link>
        </div>
      </div>

      {/* Tab content */}
      {children}
    </div>
  );
}
