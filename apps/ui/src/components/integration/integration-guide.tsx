"use client";

import { useMemo } from "react";
import Link from "next/link";
import { KeyRound, Package, Rocket, Sparkles, Terminal } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CodeBlock } from "@/components/ui/code-block";
import {
  codingAgentPrompt,
  openApiUrl,
  sdkSessionSamples,
  sessionFlowSamples,
  tokensUrl,
} from "@/lib/integration/snippets";

interface IntegrationGuideProps {
  kind: "agent" | "harness";
  id: string;
  name?: string;
}

/**
 * "Integrate" tab content shared by the Agent and Harness detail pages. This is
 * the headless-platform payoff screen: it turns a composed entity into the code
 * an external application needs to call it (create session -> send message ->
 * stream), plus a prompt to hand a coding agent.
 */
export function IntegrationGuide({ kind, id, name }: IntegrationGuideProps) {
  // Resolved at render so snippets point at this deployment's host.
  const origin = typeof window !== "undefined" ? window.location.origin : "";

  const sdkSamples = useMemo(
    () =>
      sdkSessionSamples(kind === "agent" ? { origin, agentId: id } : { origin, harnessId: id }),
    [origin, kind, id],
  );

  const flowSamples = useMemo(
    () =>
      sessionFlowSamples(kind === "agent" ? { origin, agentId: id } : { origin, harnessId: id }),
    [origin, kind, id],
  );

  const prompt = useMemo(
    () => codingAgentPrompt({ origin, kind, id, name }),
    [origin, kind, id, name],
  );

  return (
    <div className="grid gap-6 lg:grid-cols-3">
      <div className="lg:col-span-2 space-y-6">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Package className="h-4 w-4" />
              Use the SDK
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-sm text-muted-foreground">
              Everruns is headless — you compose the {kind} here and invoke it from your own code.
              The official SDKs give you typed models and SSE streaming with automatic reconnection.
              Same three steps everywhere: start a session bound to this {kind}, send a message,
              stream the response.
            </p>
            <CodeBlock samples={sdkSamples} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Terminal className="h-4 w-4" />
              Or call the HTTP API directly
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-sm text-muted-foreground">
              No SDK required — any HTTP client works. Create a session, send a message, then stream
              events.
            </p>
            <CodeBlock samples={flowSamples} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Sparkles className="h-4 w-4" />
              Let a coding agent wire it up
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-sm text-muted-foreground">
              Paste this into Claude Code, Cursor, or any coding agent to have it add a client to
              your codebase. It includes the endpoint, auth, exact flow, and a link to the OpenAPI
              spec for precise schemas.
            </p>
            <CodeBlock samples={[{ label: "Prompt", language: "text", code: prompt }]} />
          </CardContent>
        </Card>
      </div>

      <div className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <KeyRound className="h-4 w-4" />
              Authentication
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm text-muted-foreground">
            <p>
              Authenticate with a personal access token (prefix <code>evr_pat_</code>) sent as{" "}
              <code>Authorization: Bearer &lt;token&gt;</code>.
            </p>
            <Link
              href="/settings/personal-access-tokens"
              className="inline-block text-primary hover:underline"
            >
              Manage personal access tokens →
            </Link>
            <p className="break-all text-xs">
              Token page: <code>{tokensUrl(origin)}</code>
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Rocket className="h-4 w-4" />
              Going to production
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm text-muted-foreground">
            {kind === "agent" ? (
              <p>
                For external-facing or unattended use, deploy this agent as an{" "}
                <Link href="/apps/new" className="text-primary hover:underline">
                  App
                </Link>{" "}
                with a webhook, A2A, or AG-UI channel. Each channel gets its own scoped token and
                ready-made integration snippets.
              </p>
            ) : (
              <p>
                Harnesses are infrastructure. Most integrations call an{" "}
                <Link href="/agents" className="text-primary hover:underline">
                  agent
                </Link>{" "}
                that runs on this harness, or deploy one as an{" "}
                <Link href="/apps/new" className="text-primary hover:underline">
                  App
                </Link>
                . Calling a harness directly (no agent) uses the org defaults for prompt and model.
              </p>
            )}
            <p className="break-all text-xs">
              OpenAPI spec: <code>{openApiUrl(origin)}</code>
            </p>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
