"use client";

import { useUiTranslations } from "@/providers/locale-provider";
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
  const ui = useUiTranslations();
  // Resolved at render so snippets point at this deployment's host.
  const origin = typeof window !== "undefined" ? window.location.origin : "";

  const sdkSamples = useMemo(
    () => sdkSessionSamples(kind === "agent" ? { origin, agentId: id } : { origin, harnessId: id }),
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
              {ui("Use the SDK")}
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-sm text-muted-foreground">
              {ui("Everruns is headless: you compose the")} {kind}{" "}
              {ui(
                "here and invoke it from your own code. The official SDKs give you typed models and SSE streaming with automatic reconnection. Same three steps everywhere: start a session bound to this",
              )}{" "}
              {kind}
              {ui(", send a message, stream the response.")}
            </p>
            <CodeBlock samples={sdkSamples} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Terminal className="h-4 w-4" />
              {ui("Or call the HTTP API directly")}
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-sm text-muted-foreground">
              {ui(
                "No SDK required; any HTTP client works. Create a session, send a message, then stream events.",
              )}
            </p>
            <CodeBlock samples={flowSamples} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Sparkles className="h-4 w-4" />
              {ui("Let a coding agent wire it up")}
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-sm text-muted-foreground">
              {ui(
                "Paste this into Claude Code, Cursor, or any coding agent to have it add a client to your codebase. It includes the endpoint, auth, exact flow, and a link to the OpenAPI spec for precise schemas.",
              )}
            </p>
            <CodeBlock samples={[{ label: ui("Prompt"), language: "text", code: prompt }]} />
          </CardContent>
        </Card>
      </div>

      <div className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <KeyRound className="h-4 w-4" />
              {ui("Authentication")}
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm text-muted-foreground">
            <p>
              {ui("Authenticate with a personal access token (prefix")} <code>evr_pat_</code>
              {ui(") sent as")} <code>Authorization: Bearer &lt;token&gt;</code>.
            </p>
            <Link
              href="/settings/personal-access-tokens"
              className="inline-block text-primary hover:underline"
            >
              {ui("Manage personal access tokens →")}
            </Link>
            <p className="break-all text-xs">
              {ui("Token page:")} <code>{tokensUrl(origin)}</code>
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Rocket className="h-4 w-4" />
              {ui("Going to production")}
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm text-muted-foreground">
            {kind === "agent" ? (
              <p>
                {ui(
                  "Existing App channels keep working. New external integrations will use agent-owned endpoints as they become available.",
                )}
              </p>
            ) : (
              <p>
                {ui("Harnesses are infrastructure. Most integrations call an")}{" "}
                <Link href="/agents" className="text-primary hover:underline">
                  {ui("agent")}
                </Link>{" "}
                {ui(
                  "that runs on this harness. New external integrations will use agent-owned endpoints. Calling a harness directly (no agent) uses the org defaults for prompt and model.",
                )}
              </p>
            )}
            <p className="break-all text-xs">
              {ui("OpenAPI spec:")} <code>{openApiUrl(origin)}</code>
            </p>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
