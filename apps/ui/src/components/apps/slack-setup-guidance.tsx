"use client";

// Keep the Slack checklist in one component so publish-state guidance is testable.

import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Copy, CircleCheck, Circle, ExternalLink, Pencil } from "lucide-react";

interface SlackSetupGuidanceProps {
  hasSlackConfig: boolean;
  isPublished: boolean;
  webhookVerified: boolean;
  firstMessageReceived: boolean;
  webhookUrl: string;
  webhookPath: string;
  isLocalhost: boolean;
  onCreateSlackApp: () => void;
  creatingSlackApp: boolean;
  onConfigure: () => void;
}

export function SlackSetupGuidance({
  hasSlackConfig,
  isPublished,
  webhookVerified,
  firstMessageReceived,
  webhookUrl,
  webhookPath,
  isLocalhost,
  onCreateSlackApp,
  creatingSlackApp,
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
        webhookUrl={webhookUrl}
        webhookPath={webhookPath}
        isLocalhost={isLocalhost}
        onCreateSlackApp={onCreateSlackApp}
        creatingSlackApp={creatingSlackApp}
        onConfigure={onConfigure}
      />
    </>
  );
}

function StepIcon({ done }: { done: boolean }) {
  return done ? (
    <CircleCheck className="w-5 h-5 text-green-600 shrink-0" />
  ) : (
    <Circle className="w-5 h-5 text-muted-foreground shrink-0" />
  );
}

function SetupSteps({
  hasSlackConfig,
  isPublished,
  webhookVerified,
  firstMessageReceived,
  webhookUrl,
  webhookPath,
  isLocalhost,
  onCreateSlackApp,
  creatingSlackApp,
  onConfigure,
}: SlackSetupGuidanceProps) {
  const currentStep = !hasSlackConfig
    ? 1
    : !isPublished
      ? 3
      : !webhookVerified
        ? 4
        : !firstMessageReceived
          ? 5
          : 5;

  return (
    <div className="space-y-4">
      {!hasSlackConfig && (
        <p className="text-sm text-muted-foreground">
          Follow these steps to connect a Slack bot to this app.
        </p>
      )}

      <div className="flex gap-3">
        <StepIcon done={hasSlackConfig} />
        <div className="flex-1 space-y-1">
          <p
            className={`text-sm font-medium ${hasSlackConfig ? "text-muted-foreground line-through" : ""}`}
          >
            1. Create a Slack App
          </p>
          {currentStep === 1 && (
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">
                Opens Slack with a pre-filled manifest (bot scopes and settings are already
                configured). Review and click <strong>Create</strong>, then install to your
                workspace.
              </p>
              <Button size="sm" onClick={onCreateSlackApp} disabled={creatingSlackApp}>
                <ExternalLink className="w-3 h-3 mr-1" />
                {creatingSlackApp ? "Opening..." : "Create Slack App"}
              </Button>
            </div>
          )}
        </div>
      </div>

      <div className="flex gap-3">
        <StepIcon done={hasSlackConfig} />
        <div className="flex-1 space-y-1">
          <p
            className={`text-sm font-medium ${hasSlackConfig ? "text-muted-foreground line-through" : ""}`}
          >
            2. Copy credentials back
          </p>
          {currentStep === 1 && (
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">
                After creating the Slack app, copy two values back here:
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
              <Button size="sm" variant="outline" onClick={onConfigure}>
                <Pencil className="w-3 h-3 mr-1" />
                Configure
              </Button>
            </div>
          )}
        </div>
      </div>

      <div className="flex gap-3">
        <StepIcon done={isPublished} />
        <div className="flex-1 space-y-1">
          <p
            className={`text-sm font-medium ${isPublished ? "text-muted-foreground line-through" : ""}`}
          >
            3. Publish the app
          </p>
          {currentStep === 3 && (
            <p className="text-xs text-muted-foreground">
              Click the <strong>Publish</strong> button above to activate the webhook endpoint.
            </p>
          )}
        </div>
      </div>

      <div className="flex gap-3">
        <StepIcon done={webhookVerified} />
        <div className="flex-1 space-y-1">
          <p
            className={`text-sm font-medium ${webhookVerified ? "text-muted-foreground line-through" : ""}`}
          >
            4. Configure Event Subscriptions
          </p>
          {currentStep === 4 ? (
            <div className="space-y-2">
              {isLocalhost ? (
                <>
                  <p className="text-xs text-muted-foreground">
                    Slack can&apos;t reach localhost. Run{" "}
                    <code>
                      ngrok http{" "}
                      {typeof window !== "undefined" ? window.location.port || "9300" : "9300"}
                    </code>{" "}
                    then use the ngrok URL with this path as your Request URL:
                  </p>
                  <div className="flex items-center gap-2 bg-muted p-2 rounded-md">
                    <code className="text-xs flex-1 truncate">{webhookPath}</code>
                    <button
                      className="shrink-0 hover:text-foreground text-muted-foreground"
                      onClick={() => navigator.clipboard.writeText(webhookPath)}
                    >
                      <Copy className="w-3 h-3" />
                    </button>
                  </div>
                </>
              ) : (
                <>
                  <p className="text-xs text-muted-foreground">
                    In your Slack app settings, go to <strong>Event Subscriptions</strong>, enable
                    events, and paste this Request URL:
                  </p>
                  <div className="flex items-center gap-2 bg-muted p-2 rounded-md">
                    <code className="text-xs flex-1 truncate">{webhookUrl}</code>
                    <button
                      className="shrink-0 hover:text-foreground text-muted-foreground"
                      onClick={() => navigator.clipboard.writeText(webhookUrl)}
                    >
                      <Copy className="w-3 h-3" />
                    </button>
                  </div>
                </>
              )}
              <p className="text-xs text-muted-foreground">
                Then subscribe to bot events: <code>message.channels</code>,{" "}
                <code>message.groups</code>, <code>message.im</code>, <code>app_mention</code>
              </p>
            </div>
          ) : (
            <p className="text-xs text-muted-foreground">
              Add the webhook URL to your Slack app&apos;s Event Subscriptions.
            </p>
          )}
        </div>
      </div>

      <div className="flex gap-3">
        <StepIcon done={firstMessageReceived} />
        <div className="flex-1 space-y-1">
          <p
            className={`text-sm font-medium ${firstMessageReceived ? "text-muted-foreground line-through" : ""}`}
          >
            5. Invite the bot and test
          </p>
          <p className="text-xs text-muted-foreground">
            In Slack, use <code>/invite @botname</code> in a channel, then send a message.
          </p>
        </div>
      </div>
    </div>
  );
}
