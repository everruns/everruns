"use client";

import { useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { useCreateApp } from "@/hooks/use-apps";
import { usePageTitle } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AgentSelect } from "@/components/agent/agent-select";
import { HarnessSelect } from "@/components/harness/harness-select";
import { Badge } from "@/components/ui/badge";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageFooter,
  BackLink,
} from "@/components/layout";
import { Plus, Rocket } from "lucide-react";

export default function NewAppPage() {
  usePageTitle("New App", "Apps");
  const router = useRouter();
  const searchParams = useSearchParams();
  const createApp = useCreateApp();

  // Prefill harness/agent when arriving via a "Create app" shortcut from a
  // harness or agent detail page (e.g. /apps/new?harness_id=...&agent_id=...).
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [agentId, setAgentId] = useState(() => searchParams.get("agent_id") ?? "");
  const [harnessId, setHarnessId] = useState(() => searchParams.get("harness_id") ?? "");
  // Validation runs on submit, then errors stay live as fields change (mirrors
  // the design's "Create draft → inline error" flow rather than disabling the button).
  const [submitted, setSubmitted] = useState(false);

  const nameError = submitted && !name.trim();
  const harnessError = submitted && !harnessId;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitted(true);

    if (!name.trim() || !harnessId) return;

    try {
      const app = await createApp.mutateAsync({
        name: name.trim(),
        description: description.trim() || undefined,
        agent_id: agentId || undefined,
        harness_id: harnessId,
      });

      // Channels and agent identity are configured on the detail page.
      router.push(`/apps/${app.id}`);
    } catch (error) {
      console.error("Failed to create app:", error);
    }
  };

  return (
    <PageContainer fullWidth className="mx-auto max-w-3xl">
      <PageBreadcrumb items={[{ label: "Apps", href: "/apps" }, { label: "New app" }]} />

      <PageMasthead
        icon={<Rocket />}
        title="New app"
        badges={<Badge variant="accent">Draft</Badge>}
        description="Deploy an agent to a channel — Slack, AG-UI, webhook, or schedule."
      />

      <form id="app-create-form" onSubmit={handleSubmit}>
        <Card>
          <CardContent className="flex flex-col gap-6">
            <div>
              <h2 className="text-lg font-semibold tracking-tight">App details</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Name and description shown across Everruns.
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Support Bot"
                aria-invalid={nameError}
                required
              />
              {nameError ? (
                <p className="text-xs text-destructive">Enter a name to continue.</p>
              ) : (
                <p className="text-xs text-muted-foreground">
                  Shown in the apps list and channel headers.
                </p>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="description">
                Description <span className="font-normal text-muted-foreground">optional</span>
              </Label>
              <Input
                id="description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Customer support bot for #help channel"
              />
            </div>

            <hr className="border-border" />

            <div>
              <h2 className="text-lg font-semibold tracking-tight">Deployment</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Choose a harness now. You can change it or bind an agent anytime.
              </p>
            </div>

            <div className="grid grid-cols-1 gap-6 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="harness">
                  Harness <span className="text-destructive">*</span>
                </Label>
                <HarnessSelect
                  id="harness"
                  value={harnessId}
                  onValueChange={(value) => setHarnessId(value)}
                  placeholder="Select a harness"
                  className="w-full"
                />
                {harnessError ? (
                  <p className="text-xs text-destructive">Select a harness to continue.</p>
                ) : (
                  <p className="text-xs text-muted-foreground">
                    The execution harness this app runs on.
                  </p>
                )}
              </div>

              <div className="space-y-2">
                <Label htmlFor="agent">
                  Agent <span className="font-normal text-muted-foreground">optional</span>
                </Label>
                <AgentSelect
                  value={agentId}
                  onValueChange={setAgentId}
                  includeNoneOption
                  noneLabel="Choose later"
                  className="w-full"
                />
                <p className="text-xs text-muted-foreground">
                  Bind an agent now or assign one later.
                </p>
              </div>
            </div>

            <hr className="border-border" />

            <div className="flex flex-wrap items-center justify-between gap-4">
              <p className="max-w-md text-sm text-muted-foreground">
                Create the draft now — change the harness or bind an agent anytime from the detail
                page.
              </p>
              <div className="flex items-center gap-2">
                <Button type="button" variant="outline" onClick={() => router.back()}>
                  Cancel
                </Button>
                <Button type="submit" form="app-create-form" disabled={createApp.isPending}>
                  <Plus className="size-4" />
                  {createApp.isPending ? "Creating..." : "Create draft"}
                </Button>
              </div>
            </div>

            {createApp.error && (
              <p className="text-sm text-destructive">Error: {createApp.error.message}</p>
            )}
          </CardContent>
        </Card>
      </form>

      <PageFooter>
        <BackLink href="/apps">Back to Apps</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
