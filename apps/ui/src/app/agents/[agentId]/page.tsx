"use client";

import { use } from "react";
import { useHarness, useSessions, useCreateSession } from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { ArrowLeft, Plus, MessageSquare } from "lucide-react";

export default function HarnessDetailPage({
  params,
}: {
  params: Promise<{ harnessId: string }>;
}) {
  const { harnessId } = use(params);
  const router = useRouter();
  const { data: harness, isLoading: harnessLoading } = useHarness(harnessId);
  const { data: sessions, isLoading: sessionsLoading } = useSessions(harnessId);
  const createSession = useCreateSession();

  const handleNewSession = async () => {
    try {
      const session = await createSession.mutateAsync({
        harnessId,
        request: {},
      });
      router.push(`/harnesses/${harnessId}/sessions/${session.id}`);
    } catch (error) {
      console.error("Failed to create session:", error);
    }
  };

  if (harnessLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!harness) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">Harness not found</div>
        <Link href="/harnesses" className="text-blue-500 hover:underline">
          Back to harnesses
        </Link>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/harnesses"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Harnesses
      </Link>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            {harness.display_name}
            <Badge
              variant={harness.status === "active" ? "default" : "secondary"}
            >
              {harness.status}
            </Badge>
          </h1>
          <p className="text-muted-foreground font-mono text-sm">
            {harness.slug}
          </p>
        </div>
        <Button onClick={handleNewSession} disabled={createSession.isPending}>
          <Plus className="w-4 h-4 mr-2" />
          {createSession.isPending ? "Creating..." : "New Session"}
        </Button>
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
          <Card>
            <CardHeader>
              <CardTitle>System Prompt</CardTitle>
            </CardHeader>
            <CardContent>
              <pre className="whitespace-pre-wrap text-sm bg-muted p-4 rounded-md">
                {harness.system_prompt}
              </pre>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Sessions</CardTitle>
            </CardHeader>
            <CardContent>
              {sessionsLoading ? (
                <div className="space-y-2">
                  <Skeleton className="h-12 w-full" />
                  <Skeleton className="h-12 w-full" />
                </div>
              ) : sessions?.length === 0 ? (
                <p className="text-center py-8 text-muted-foreground">
                  No sessions yet. Start a new session to begin chatting.
                </p>
              ) : (
                <div className="space-y-2">
                  {sessions?.map((session) => (
                    <Link
                      key={session.id}
                      href={`/harnesses/${harnessId}/sessions/${session.id}`}
                      className="flex items-center justify-between p-3 rounded-md border hover:bg-muted transition-colors"
                    >
                      <div className="flex items-center gap-3">
                        <MessageSquare className="w-4 h-4 text-muted-foreground" />
                        <div>
                          <p className="font-medium">
                            {session.title || `Session ${session.id.slice(0, 8)}`}
                          </p>
                          <p className="text-xs text-muted-foreground">
                            {new Date(session.created_at).toLocaleString()}
                          </p>
                        </div>
                      </div>
                      {session.finished_at && (
                        <Badge variant="outline">Completed</Badge>
                      )}
                    </Link>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>
        </div>

        <div className="space-y-6">
          <Card>
            <CardHeader>
              <CardTitle>Configuration</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              {harness.description && (
                <div>
                  <p className="text-sm font-medium">Description</p>
                  <p className="text-sm text-muted-foreground">
                    {harness.description}
                  </p>
                </div>
              )}

              {harness.temperature !== null && (
                <div>
                  <p className="text-sm font-medium">Temperature</p>
                  <p className="text-sm text-muted-foreground">
                    {harness.temperature}
                  </p>
                </div>
              )}

              {harness.max_tokens !== null && (
                <div>
                  <p className="text-sm font-medium">Max Tokens</p>
                  <p className="text-sm text-muted-foreground">
                    {harness.max_tokens}
                  </p>
                </div>
              )}

              {harness.tags.length > 0 && (
                <div>
                  <p className="text-sm font-medium mb-2">Tags</p>
                  <div className="flex flex-wrap gap-1">
                    {harness.tags.map((tag) => (
                      <Badge key={tag} variant="outline">
                        {tag}
                      </Badge>
                    ))}
                  </div>
                </div>
              )}

              <div>
                <p className="text-sm font-medium">Created</p>
                <p className="text-sm text-muted-foreground">
                  {new Date(harness.created_at).toLocaleString()}
                </p>
              </div>

              <div>
                <p className="text-sm font-medium">Updated</p>
                <p className="text-sm text-muted-foreground">
                  {new Date(harness.updated_at).toLocaleString()}
                </p>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
