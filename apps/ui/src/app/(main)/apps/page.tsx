"use client";

import { useState } from "react";
import { useApps, usePublishApp, useUnpublishApp } from "@/hooks/use-apps";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { Plus, Rocket, Globe, GlobeLock, Copy } from "lucide-react";
import { ArchiveFilter } from "@/components/archive-filter";
import { CopyButton } from "@/components/ui/copy-button";
import Link from "next/link";
import type { App } from "@/lib/api/types";
import { ExperimentalPageBadge } from "@/components/ui/experimental-badge";
import { getEntityNameClassName, getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";

export default function AppsPage() {
  const [showArchived, setShowArchived] = useState(false);
  const { data: apps, isLoading, error } = useApps({ includeArchived: showArchived });

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold flex items-center gap-3">
          Apps
          <ExperimentalPageBadge />
        </h1>
        <div className="flex items-center gap-2">
          <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
          <Link href="/apps/new">
            <Button variant="accent">
              <Plus className="w-4 h-4 mr-2" />
              New App
            </Button>
          </Link>
        </div>
      </div>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={apps}
        skeletonCount={3}
        emptyState={<EmptyState />}
      >
        {(items) => (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {items.map((app) => (
              <AppCard key={app.id} app={app} />
            ))}
          </div>
        )}
      </QueryStateWrapper>
    </div>
  );
}

function AppCard({ app }: { app: App }) {
  const publishApp = usePublishApp();
  const unpublishApp = useUnpublishApp();
  const isPublished = app.status === "published";
  const isArchived = app.status === "archived";
  const webhookUrl =
    typeof window !== "undefined"
      ? `${window.location.origin}/api/v1/apps/${app.id}/slack/events`
      : `/api/v1/apps/${app.id}/slack/events`;

  return (
    <Link href={`/apps/${app.id}`}>
      <Card className="cursor-pointer hover:border-accent/50 transition-colors">
        <CardHeader className="pb-2">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Rocket className="w-5 h-5 text-muted-foreground" />
              <h3 className={`font-semibold text-lg ${getEntityNameClassName(app.status)}`}>
                {app.name}
              </h3>
              <CopyButton value={app.id} />
            </div>
            <Badge variant={getEntityStatusBadgeVariant(app.status)}>{app.status}</Badge>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          {app.description && <p className="text-sm text-muted-foreground">{app.description}</p>}
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Badge variant="outline" className="text-xs">
              Slack
            </Badge>
          </div>

          {isPublished && (
            <div className="flex items-center gap-1 text-xs text-muted-foreground bg-muted p-2 rounded">
              <Globe className="w-3 h-3 shrink-0" />
              <span className="truncate font-mono">{webhookUrl}</span>
              <button
                className="shrink-0 ml-1 hover:text-foreground"
                onClick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  navigator.clipboard.writeText(webhookUrl);
                }}
              >
                <Copy className="w-3 h-3" />
              </button>
            </div>
          )}

          {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions */}
          <div
            className="flex gap-2 pt-2"
            role="group"
            onClick={(e) => {
              e.preventDefault();
              e.stopPropagation();
            }}
          >
            {isPublished ? (
              <Button
                variant="outline"
                size="sm"
                onClick={() => unpublishApp.mutate(app.id)}
                disabled={unpublishApp.isPending || isArchived}
              >
                <GlobeLock className="w-3 h-3 mr-1" />
                Unpublish
              </Button>
            ) : (
              <Button
                variant="default"
                size="sm"
                onClick={() => publishApp.mutate(app.id)}
                disabled={publishApp.isPending || isArchived}
              >
                <Globe className="w-3 h-3 mr-1" />
                Publish
              </Button>
            )}
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-col items-center justify-center py-12 text-center">
      <Rocket className="w-12 h-12 text-muted-foreground mb-4" />
      <h3 className="text-lg font-semibold mb-2">No Apps Yet</h3>
      <p className="text-muted-foreground mb-4 max-w-md">
        Apps deploy your agents to channels like Slack. Create an app to connect an agent to a Slack
        workspace.
      </p>
      <Link href="/apps/new">
        <Button variant="accent">
          <Plus className="w-4 h-4 mr-2" />
          Create Your First App
        </Button>
      </Link>
    </div>
  );
}
