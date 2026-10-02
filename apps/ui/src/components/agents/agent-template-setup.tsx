"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Check, ExternalLink } from "lucide-react";
import { useAgent, useUpdateAgent } from "@/hooks/use-agents";
import { useAgentExamples } from "@/hooks/use-agent-examples";
import { useCreateAgentTrigger } from "@/hooks/use-agent-triggers";
import {
  useAgentGitHub,
  useConnectAgentGitHub,
  useGitHubRepositories,
} from "@/hooks/use-agent-github";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  applyTemplateSettings,
  buildTemplateTrigger,
  canFinishSetup,
  defaultSettingValues,
} from "@/lib/agent-template-setup";
import type { AgentExampleSetup } from "@/lib/api/agent-examples";

function Step({
  number,
  title,
  done,
  children,
}: {
  number: number;
  title: string;
  done?: boolean;
  children: React.ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <span className="text-muted-foreground">{number}.</span> {title}
          {done && <Check className="size-4 text-primary" aria-label="done" />}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">{children}</CardContent>
    </Card>
  );
}

/// Walks a freshly imported template through what it needs before it can run
/// unattended: its own GitHub App, a repository, its settings, its trigger.
export function AgentTemplateSetup({
  agentId,
  templateName,
}: {
  agentId: string;
  templateName: string;
}) {
  const router = useRouter();
  const { data: agent } = useAgent(agentId);
  const { data: examples, isLoading: examplesLoading } = useAgentExamples();
  const example = examples?.find((candidate) => candidate.name === templateName);
  const setup: AgentExampleSetup | undefined = example?.setup;

  const returnTo = `/agents/${agentId}/setup?template=${encodeURIComponent(templateName)}`;
  const { data: github } = useAgentGitHub(agentId);
  const connect = useConnectAgentGitHub(agentId, returnTo);
  const connected = !!github?.connected;
  const { data: repos = [] } = useGitHubRepositories(github?.identity_id, connected);
  const updateAgent = useUpdateAgent();
  const createTrigger = useCreateAgentTrigger(agentId);

  const [picked, setRepository] = useState("");
  const [overrides, setOverrides] = useState<Record<string, boolean>>({});
  const [error, setError] = useState<string | null>(null);
  // A single installed repository is the obvious pick.
  const repository = picked || (repos.length === 1 ? repos[0].full_name : "");
  const values = useMemo(
    () => ({ ...defaultSettingValues(setup?.settings ?? []), ...overrides }),
    [setup, overrides],
  );

  const ready = useMemo(
    () => !!setup && canFinishSetup(setup, connected, repository),
    [setup, connected, repository],
  );

  if (examplesLoading) return <p className="text-sm text-muted-foreground">Loading template…</p>;
  if (!example || !setup) {
    return (
      <p className="text-sm text-muted-foreground">
        This template has no guided setup.{" "}
        <Link className="text-primary underline" href={`/agents/${agentId}`}>
          Open the agent
        </Link>
      </p>
    );
  }

  const finish = async () => {
    if (!agent) return;
    setError(null);
    try {
      const capabilities = applyTemplateSettings(agent.capabilities ?? [], setup.settings, values);
      if (capabilities) {
        await updateAgent.mutateAsync({ agentId, request: { capabilities } });
      }
      await createTrigger.mutateAsync(buildTemplateTrigger(setup, repository));
      router.push(`/agents/${agentId}?tab=integrations`);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Unable to finish setup.");
    }
  };

  let step = 0;
  const busy = updateAgent.isPending || createTrigger.isPending;
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{example.description}</p>

      {setup.connect_github && (
        <Step number={++step} title="Connect GitHub" done={connected}>
          {connected ? (
            <p className="text-sm">
              Installed as <span className="font-medium">{github?.account ?? "GitHub App"}</span>.
            </p>
          ) : (
            <>
              <p className="text-sm text-muted-foreground">
                The agent gets its own GitHub App, installed on the repositories you choose. It
                reads code and posts as its own bot.
              </p>
              <Button size="sm" onClick={() => connect.mutate()} disabled={connect.isPending}>
                {github?.app_created ? "Finish installing" : "Connect GitHub"}
              </Button>
            </>
          )}
        </Step>
      )}

      <Step
        number={++step}
        title="Pick a repository"
        done={canFinishSetup(setup, true, repository)}
      >
        {repos.length > 0 ? (
          <Select value={repository} onValueChange={setRepository}>
            <SelectTrigger aria-label="Repository" className="w-full">
              <SelectValue placeholder="Choose a repository" />
            </SelectTrigger>
            <SelectContent>
              {repos.map((repo) => (
                <SelectItem key={repo.full_name} value={repo.full_name}>
                  {repo.full_name}
                  {repo.private ? "" : " (public)"}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : (
          <div className="space-y-1">
            <Label htmlFor="template-repository">Repository</Label>
            <Input
              id="template-repository"
              placeholder="owner/name"
              value={repository}
              onChange={(event) => setRepository(event.target.value.trim())}
            />
          </div>
        )}
      </Step>

      {(setup.settings.length > 0 || setup.connections.length > 0) && (
        <Step number={++step} title="Settings">
          {setup.settings.map((setting) => (
            <div key={setting.key} className="flex items-start justify-between gap-4">
              <div className="space-y-1">
                <Label htmlFor={`template-setting-${setting.key}`}>{setting.label}</Label>
                <p className="text-xs text-muted-foreground">{setting.description}</p>
              </div>
              <Switch
                id={`template-setting-${setting.key}`}
                checked={values[setting.key]}
                onCheckedChange={(on) => setOverrides({ ...overrides, [setting.key]: on })}
              />
            </div>
          ))}
          {setup.connections.map((provider) => (
            <p key={provider} className="text-sm text-muted-foreground">
              Also needs a <span className="font-medium">{provider}</span> connection on the
              agent&apos;s service account
              {github?.identity_id && (
                <>
                  {" "}
                  (
                  <Link
                    className="text-primary underline"
                    href={`/virtual-users/${github.identity_id}`}
                  >
                    open it <ExternalLink className="inline size-3" />
                  </Link>
                  )
                </>
              )}
              .
            </p>
          ))}
        </Step>
      )}

      <div className="flex items-center gap-2">
        <Button onClick={finish} disabled={!ready || busy || !agent}>
          {busy ? "Creating trigger…" : "Create trigger"}
        </Button>
        <Button variant="outline" onClick={() => router.push(`/agents/${agentId}`)}>
          Skip for now
        </Button>
      </div>
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  );
}
