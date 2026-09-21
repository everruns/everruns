"use client";

// Keep the Slack checklist in one component so publish-state guidance is testable.
//
// Publish comes first (EVE-970). Slack verifies `request_url` at the moment the
// manifest is saved, and the generated manifest now carries `event_subscriptions`,
// so the webhook has to be answering before the Slack app is created from it.
// That ordering is what removes the old hand-editing step.

import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Copy, CircleCheck, Circle, ExternalLink, Pencil } from "lucide-react";

interface SlackSetupGuidanceProps {
  hasSlackConfig: boolean;
  isPublished: boolean;
  webhookVerified: boolean;
  firstMessageReceived: boolean;
  manifestRequestUrl: string | null;
  manifestLoading: boolean;
  canCreateSlackApp: boolean;
  agentSurfaceEnabled: boolean;
  onCreateSlackApp: () => void;
  onConfigure?: () => void;
}

export function SlackSetupGuidance({
  hasSlackConfig,
  isPublished,
  webhookVerified,
  firstMessageReceived,
  manifestRequestUrl,
  manifestLoading,
  canCreateSlackApp,
  agentSurfaceEnabled,
  onCreateSlackApp,
  onConfigure,
}: SlackSetupGuidanceProps) {
  return (
    <>
      {hasSlackConfig && <Separator />}
      <SetupSteps
        hasSlackConfig={hasSlackConfig}
        isPublished={isPublished}
        webhookVerified={webhookVerified}
        firstMessageReceived={firstMessageReceived}
        manifestRequestUrl={manifestRequestUrl}
        manifestLoading={manifestLoading}
        canCreateSlackApp={canCreateSlackApp}
        agentSurfaceEnabled={agentSurfaceEnabled}
        onCreateSlackApp={onCreateSlackApp}
        onConfigure={onConfigure}
      />
      {hasSlackConfig && <AgentSurfaceNotice agentSurfaceEnabled={agentSurfaceEnabled} />}
    </>
  );
}

/// The agent surface needs a new OAuth scope, so it is never a pure config flip.
/// Saying so up front beats offering a toggle that half-works.
function AgentSurfaceNotice({ agentSurfaceEnabled }: { agentSurfaceEnabled: boolean }) {
  return (
    <div className="mt-4 border-t pt-4 space-y-1">
      <p className="text-sm font-medium">
        {agentSurfaceEnabled ? "Agent surface enabled" : "Agent surface available"}
      </p>
      {agentSurfaceEnabled ? (
        <p className="text-xs text-muted-foreground">
          This app also serves Slack&apos;s agent pane. If the pane has not appeared, the Slack app
          still needs reinstalling to pick up the <code>assistant:write</code> scope. Channel
          threads are unaffected either way.
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          Enabling it gives this app Slack&apos;s agent pane <em>alongside</em> its channel bot —
          nothing about channel replies changes. It requires the <code>assistant:write</code> scope,
          which a config change cannot grant on its own: you will need to update the Slack app from
          a fresh manifest <strong>and reinstall it</strong> to your workspace.
        </p>
      )}
    </div>
  );
}

function StepIcon({ done }: { done: boolean }) {
  return done ? (
    <CircleCheck className="w-5 h-5 text-success shrink-0" />
  ) : (
    <Circle className="w-5 h-5 text-muted-foreground shrink-0" />
  );
}

function CopyableValue({ value }: { value: string }) {
  return (
    <div className="flex items-center gap-2 bg-muted p-2">
      <code className="text-xs flex-1 truncate">{value}</code>
      <button
        className="shrink-0 hover:text-foreground text-muted-foreground"
        onClick={() => navigator.clipboard.writeText(value)}
        aria-label="Copy"
      >
        <Copy className="w-3 h-3" />
      </button>
    </div>
  );
}

function SetupSteps({
  hasSlackConfig,
  isPublished,
  webhookVerified,
  firstMessageReceived,
  manifestRequestUrl,
  manifestLoading,
  canCreateSlackApp,
  onCreateSlackApp,
  onConfigure,
}: SlackSetupGuidanceProps) {
  const currentStep = !isPublished ? 1 : !hasSlackConfig ? 2 : !firstMessageReceived ? 4 : 4;

  return (
    <div className="space-y-4">
      {!hasSlackConfig && (
        <p className="text-sm text-muted-foreground">
          Follow these steps to connect a Slack bot to this endpoint.
        </p>
      )}

      <div className="flex gap-3">
        <StepIcon done={isPublished} />
        <div className="flex-1 space-y-1">
          <p
            className={`text-sm font-medium ${isPublished ? "text-muted-foreground line-through" : ""}`}
          >
            1. Publish the endpoint
          </p>
          {currentStep === 1 && (
            <p className="text-xs text-muted-foreground">
              Click the <strong>Publish</strong> button above to activate the webhook endpoint.
              Slack checks this URL the moment you create the app, so it has to be live first.
            </p>
          )}
        </div>
      </div>

      <div className="flex gap-3">
        <StepIcon done={hasSlackConfig} />
        <div className="flex-1 space-y-1">
          <p
            className={`text-sm font-medium ${hasSlackConfig ? "text-muted-foreground line-through" : ""}`}
          >
            2. Create a Slack app
          </p>
          {currentStep === 1 && (
            <p className="text-xs text-muted-foreground">
              Available once the endpoint is published — the manifest points Slack at this
              endpoint&apos;s webhook, and Slack rejects a URL that does not answer yet.
            </p>
          )}
          {currentStep === 2 && (
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">
                Opens Slack with a pre-filled manifest. Bot scopes <em>and</em> event subscriptions
                are already set, so there is nothing to configure by hand. Review and click{" "}
                <strong>Create</strong>, then install it to your workspace.
              </p>
              {manifestLoading ? (
                <p className="text-xs text-muted-foreground">Loading the endpoint manifest…</p>
              ) : canCreateSlackApp && manifestRequestUrl ? (
                <>
                  <p className="text-xs text-muted-foreground">
                    The manifest subscribes to <code>app_mention</code>,{" "}
                    <code>message.channels</code>, <code>message.groups</code>,{" "}
                    <code>message.im</code> and <code>message.mpim</code> at this Request URL:
                  </p>
                  <CopyableValue value={manifestRequestUrl} />
                </>
              ) : (
                <p className="text-xs text-muted-foreground">
                  Slack validates the manifest&apos;s Request URL during app creation. Set{" "}
                  <code>PUBLIC_APP_URL</code> to a public HTTPS origin, restart Everruns, and reload
                  this page before creating the Slack app.
                </p>
              )}
              <Button
                size="sm"
                onClick={onCreateSlackApp}
                disabled={manifestLoading || !canCreateSlackApp}
              >
                <ExternalLink className="w-3 h-3 mr-1" />
                Create Slack app
              </Button>
            </div>
          )}
          {hasSlackConfig && !webhookVerified && (
            <p className="text-xs text-muted-foreground">
              Waiting for Slack&apos;s first call to the Request URL.
            </p>
          )}
        </div>
      </div>

      <div className="flex gap-3">
        <StepIcon done={hasSlackConfig} />
        <div className="flex-1 space-y-1">
          <p
            className={`text-sm font-medium ${hasSlackConfig ? "text-muted-foreground line-through" : ""}`}
          >
            3. Copy credentials back
          </p>
          {currentStep === 2 && (
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">
                After creating the Slack app, open Configure and copy two values into this endpoint:
              </p>
              <ul className="text-xs text-muted-foreground list-disc pl-4 space-y-1">
                <li>
                  <strong>Signing Secret</strong> - Slack app &rarr; Basic Information &rarr; App
                  Credentials
                </li>
                <li>
                  <strong>Bot Token</strong> (<code>xoxb-...</code>) - Slack app &rarr; OAuth &amp;
                  Permissions
                </li>
              </ul>
              {onConfigure && (
                <Button size="sm" variant="outline" onClick={onConfigure}>
                  <Pencil className="w-3 h-3 mr-1" />
                  Configure
                </Button>
              )}
            </div>
          )}
        </div>
      </div>

      <div className="flex gap-3">
        <StepIcon done={firstMessageReceived} />
        <div className="flex-1 space-y-1">
          <p
            className={`text-sm font-medium ${firstMessageReceived ? "text-muted-foreground line-through" : ""}`}
          >
            4. Invite the bot and test
          </p>
          <p className="text-xs text-muted-foreground">
            In Slack, use <code>/invite @botname</code> in a channel, then send a message.
          </p>
        </div>
      </div>
    </div>
  );
}
