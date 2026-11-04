"use client";

// External agent: an agent whose turns run on the customer's own HTTPS endpoint
// (knowledge/operations/custom-workers.md). The endpoint owns the system prompt,
// model and tools, so the agent page hides those settings while one is bound and
// shows the endpoint in the prompt pane's place (knowledge/ui/agent-page.md).
//
// Decisions:
// - Setup is a More row plus this side sheet, never a card on the page: most
//   agents never use it, and the row is hidden where the installation does not
//   allow it (`available` from the status endpoint).
// - The secret is generated in the browser and only leaves it on Verify and
//   connect. The person installs it in the endpoint first; the server verifies
//   ownership with a signed challenge before anything changes, so updating a
//   connection keeps the current one until the new one is verified.
// - Disconnect keeps the agent bound: new messages queue, Everruns does not run
//   them itself. The sheet states that before the button.

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { CircleAlert, Copy, ExternalLink, Power, RefreshCw, Server } from "lucide-react";
import { useOrgScopedQuery } from "@/hooks/create-crud-hooks";
import { api } from "@/lib/api/client";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import { useConfirmation } from "@/components/ui/confirmation-provider";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { cn } from "@/lib/utils";

export const EXTERNAL_AGENT_GUIDE_URL = "https://everruns.com/docs/how-to/cloudflare-agent/";

export type ExternalAgentEndpoint = {
  endpoint_url: string;
  enabled: boolean;
  last_error: string | null;
  last_invoked_at: string | null;
};

export type ExternalAgentStatus = {
  available: boolean;
  endpoint: ExternalAgentEndpoint | null;
};

const statusPath = (agentId: string) => `/v1/agents/${encodeURIComponent(agentId)}/external-worker`;
export const externalAgentKey = (agentId: string) => ["external-agent", agentId];

export function useExternalAgent(agentId: string, enabled = true) {
  return useOrgScopedQuery({
    queryKey: externalAgentKey(agentId),
    queryFn: async () => (await api.get<ExternalAgentStatus>(statusPath(agentId))).data,
    enabled,
    // Last run and last error change while the endpoint serves turns.
    refetchInterval: 15000,
  });
}

/** 48 URL-safe characters from the browser's CSPRNG. */
export function generateEndpointSecret(): string {
  const bytes = new Uint8Array(36);
  crypto.getRandomValues(bytes);
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

/** One-line value for the More row. */
export function externalAgentSummary(
  endpoint: ExternalAgentEndpoint | null | undefined,
  ui: (message: string) => string,
): string {
  if (!endpoint) return ui("Not connected");
  return endpoint.enabled ? ui("Connected") : ui("Disconnected");
}

function useRelativeTime() {
  const { displayLocale } = useLocale();
  const ui = useUiTranslations();
  return (value: string | null) => {
    if (!value) return ui("Never");
    const seconds = Math.round((new Date(value).getTime() - Date.now()) / 1000);
    const format = new Intl.RelativeTimeFormat(displayLocale, {
      numeric: "auto",
    });
    const abs = Math.abs(seconds);
    if (abs < 60) return format.format(seconds, "second");
    if (abs < 3600) return format.format(Math.round(seconds / 60), "minute");
    if (abs < 86400) return format.format(Math.round(seconds / 3600), "hour");
    return format.format(Math.round(seconds / 86400), "day");
  };
}

function StatusDot({ enabled }: { enabled: boolean }) {
  const ui = useUiTranslations();
  return (
    <span className="inline-flex items-center gap-1.5 text-xs font-normal text-muted-foreground">
      <span
        aria-hidden
        className={cn("size-1.5 rounded-full", enabled ? "bg-success" : "bg-warning")}
      />
      {enabled ? ui("Connected") : ui("Disconnected")}
    </span>
  );
}

function Facts({ endpoint }: { endpoint: ExternalAgentEndpoint }) {
  const ui = useUiTranslations();
  const relative = useRelativeTime();
  return (
    <dl className="grid grid-cols-[8rem_1fr] gap-x-4 gap-y-2 text-[13px]">
      <dt className="text-muted-foreground">{ui("URL")}</dt>
      <dd className="min-w-0 break-all font-mono text-xs leading-5">{endpoint.endpoint_url}</dd>
      <dt className="text-muted-foreground">{ui("Last run")}</dt>
      <dd>{relative(endpoint.last_invoked_at)}</dd>
      {!endpoint.enabled && (
        <>
          <dt className="text-muted-foreground">{ui("Queue")}</dt>
          <dd>{ui("New messages wait until an endpoint is connected again.")}</dd>
        </>
      )}
    </dl>
  );
}

function LastError({ endpoint }: { endpoint: ExternalAgentEndpoint }) {
  const ui = useUiTranslations();
  if (!endpoint.last_error) return null;
  return (
    <Notice variant="destructive" icon={<CircleAlert className="size-4" />}>
      <NoticeTitle>{ui("The last call failed")}</NoticeTitle>
      <NoticeDescription>
        {ui("Open the session's trace for details.")}{" "}
        <span className="break-all font-mono text-xs">{endpoint.last_error}</span>
      </NoticeDescription>
    </Notice>
  );
}

/** Replaces the system prompt pane on the Agent tab while an endpoint is bound. */
export function ExternalAgentPane({
  endpoint,
  onManage,
  className,
}: {
  endpoint: ExternalAgentEndpoint;
  onManage: () => void;
  className?: string;
}) {
  const ui = useUiTranslations();
  return (
    <section aria-label={ui("External agent")} className={cn("flex min-w-0 flex-col", className)}>
      <div className="flex flex-wrap items-center gap-3 border-b px-5 py-3">
        <h2 className="text-sm font-semibold">{ui("Endpoint")}</h2>
        <StatusDot enabled={endpoint.enabled} />
        <Button variant="outline" size="sm" className="ml-auto" onClick={onManage}>
          {endpoint.enabled ? ui("Manage") : ui("Reconnect")}
        </Button>
      </div>
      <div className="flex min-h-64 flex-col gap-5 p-5">
        <Facts endpoint={endpoint} />
        <LastError endpoint={endpoint} />
        <p className="text-[13px] text-muted-foreground">
          {ui(
            "Your endpoint runs every turn with its own system prompt, model and tools. Everruns keeps the sessions, channels, traces and usage for this agent.",
          )}
        </p>
      </div>
    </section>
  );
}

function SetupForm({
  agentId,
  endpoint,
  onDone,
  onCancel,
}: {
  agentId: string;
  endpoint: ExternalAgentEndpoint | null;
  onDone: () => void;
  onCancel?: () => void;
}) {
  const ui = useUiTranslations();
  const cache = useQueryClient();
  const [url, setUrl] = useState(endpoint?.endpoint_url ?? "");
  const [secret, setSecret] = useState("");
  const [copied, setCopied] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function connect() {
    setPending(true);
    setError(null);
    try {
      await api.post(statusPath(agentId), {
        endpoint_url: url,
        endpoint_secret: secret,
      });
      setSecret("");
      await cache.invalidateQueries({ queryKey: externalAgentKey(agentId) });
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : ui("Could not connect the endpoint."));
    } finally {
      setPending(false);
    }
  }

  return (
    <form
      id="external-agent-form"
      className="flex flex-col gap-5"
      onSubmit={(event) => {
        event.preventDefault();
        void connect();
      }}
    >
      <Step n={1} title={ui("Deploy your endpoint")}>
        <p className="text-xs text-muted-foreground">
          {ui("Any HTTPS service that implements the endpoint protocol.")}{" "}
          <a
            href={EXTERNAL_AGENT_GUIDE_URL}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1 underline underline-offset-2"
          >
            {ui("Cloudflare agent guide")}
            <ExternalLink className="size-3" />
          </a>
        </p>
      </Step>
      <Step n={2} title={ui("Install an endpoint secret")} htmlFor="external-agent-secret">
        <div className="flex gap-2">
          <Input
            id="external-agent-secret"
            type="password"
            autoComplete="new-password"
            required
            minLength={32}
            maxLength={256}
            value={secret}
            onChange={(event) => {
              setSecret(event.target.value);
              setCopied(false);
            }}
            className="font-mono"
            aria-describedby="external-agent-secret-hint"
          />
          <Button
            type="button"
            variant="outline"
            size="icon"
            aria-label={copied ? ui("Copied") : ui("Copy secret")}
            disabled={!secret}
            onClick={() => {
              void navigator.clipboard?.writeText(secret).then(() => setCopied(true));
            }}
          >
            <Copy className="size-4" />
          </Button>
          <Button
            type="button"
            variant="outline"
            onClick={() => {
              setSecret(generateEndpointSecret());
              setCopied(false);
            }}
          >
            <RefreshCw className="size-4" />
            {ui("Generate")}
          </Button>
        </div>
        <p id="external-agent-secret-hint" className="text-xs text-muted-foreground">
          {ui(
            "Set it in your endpoint's environment before you connect, for example with wrangler secret put EVERRUNS_ENDPOINT_SECRET. At least 32 characters. Everruns stores it encrypted and does not show it again.",
          )}
        </p>
      </Step>
      <Step n={3} title={ui("Endpoint URL")} htmlFor="external-agent-url">
        <Input
          id="external-agent-url"
          type="url"
          required
          placeholder="https://my-agent.example.workers.dev"
          value={url}
          onChange={(event) => setUrl(event.target.value)}
          className="font-mono"
        />
      </Step>
      <p className="text-[13px] text-muted-foreground">
        {ui(
          "Everruns sends a signed challenge to check that the endpoint holds the secret. After you connect, messages and session history for this agent go to the endpoint. Running turns must finish first.",
        )}
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button type="submit" loading={pending}>
          {ui("Verify and connect")}
        </Button>
        {onCancel && (
          <Button type="button" variant="outline" onClick={onCancel} disabled={pending}>
            {ui("Cancel")}
          </Button>
        )}
      </div>
    </form>
  );
}

function Step({
  n,
  title,
  htmlFor,
  children,
}: {
  n: number;
  title: string;
  htmlFor?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex gap-3">
      <span
        aria-hidden
        className="flex size-5 shrink-0 items-center justify-center border text-xs text-muted-foreground"
      >
        {n}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        {htmlFor ? (
          <Label htmlFor={htmlFor} className="text-sm font-semibold">
            {title}
          </Label>
        ) : (
          <span className="text-sm font-semibold">{title}</span>
        )}
        {children}
      </div>
    </div>
  );
}

/** The More row's side sheet: setup, status, update and disconnect. */
export function ExternalAgentSheet({
  agentId,
  open,
  onOpenChange,
  status,
  canManage,
}: {
  agentId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  status: ExternalAgentStatus | undefined;
  canManage: boolean;
}) {
  const ui = useUiTranslations();
  const cache = useQueryClient();
  const confirm = useConfirmation();
  const [updating, setUpdating] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const endpoint = status?.endpoint ?? null;
  const showForm = canManage && (!endpoint || updating);

  function close(next: boolean) {
    if (!next) {
      setUpdating(false);
      setError(null);
    }
    onOpenChange(next);
  }

  async function disconnect() {
    if (
      !(await confirm(
        ui(
          "Disconnect the endpoint? New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.",
        ),
      ))
    )
      return;
    setPending(true);
    setError(null);
    try {
      await api.delete(statusPath(agentId));
      await cache.invalidateQueries({ queryKey: externalAgentKey(agentId) });
    } catch (e) {
      setError(e instanceof Error ? e.message : ui("Could not disconnect the endpoint."));
    } finally {
      setPending(false);
    }
  }

  return (
    <Drawer open={open} onOpenChange={close}>
      <DrawerContent scopeToPage className="gap-0 overflow-y-auto p-0 sm:max-w-xl">
        <DrawerHeader className="border-b p-5 pr-12">
          <DrawerTitle>{ui("External agent")}</DrawerTitle>
          <DrawerDescription>
            {endpoint
              ? ui("Your endpoint runs this agent's turns.")
              : ui("Run this agent's turns on your own HTTPS endpoint.")}
          </DrawerDescription>
        </DrawerHeader>
        <div className="flex flex-1 flex-col gap-6 p-5">
          {!endpoint && (
            <p className="text-[13px] text-muted-foreground">
              {ui(
                "Your code runs each turn with its own system prompt, model and tools. Sessions, Playground, Trace and channels keep working. While an endpoint is connected, the page hides the settings it owns.",
              )}
            </p>
          )}
          {endpoint && !updating && (
            <>
              <div className="flex items-center gap-2 text-sm font-semibold">
                <Server className="size-4 text-muted-foreground" />
                {ui("Status")}
                <StatusDot enabled={endpoint.enabled} />
              </div>
              <Facts endpoint={endpoint} />
              <LastError endpoint={endpoint} />
              {canManage && (
                <>
                  <div className="flex flex-col gap-2">
                    <span className="text-xs font-medium text-muted-foreground">
                      {endpoint.enabled
                        ? ui("Change the endpoint or rotate the secret")
                        : ui("Reconnect")}
                    </span>
                    <p className="text-xs text-muted-foreground">
                      {ui(
                        "Install the new secret in your endpoint first, then verify again. The current connection keeps working until the new one is verified.",
                      )}
                    </p>
                    <div>
                      <Button variant="outline" onClick={() => setUpdating(true)}>
                        <RefreshCw className="size-4" />
                        {endpoint.enabled ? ui("Update connection") : ui("Reconnect")}
                      </Button>
                    </div>
                  </div>
                  {endpoint.enabled && (
                    <div className="flex flex-col gap-2 border-t pt-5">
                      <span className="text-xs font-medium text-muted-foreground">
                        {ui("Disconnect")}
                      </span>
                      <p className="text-xs text-muted-foreground">
                        {ui(
                          "New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.",
                        )}
                      </p>
                      <div>
                        <Button
                          variant="outline"
                          className="text-destructive"
                          loading={pending}
                          onClick={() => void disconnect()}
                        >
                          <Power className="size-4" />
                          {ui("Disconnect endpoint")}
                        </Button>
                      </div>
                    </div>
                  )}
                </>
              )}
            </>
          )}
          {showForm && (
            <SetupForm
              key={endpoint?.endpoint_url ?? "new"}
              agentId={agentId}
              endpoint={endpoint}
              onDone={() => setUpdating(false)}
              onCancel={endpoint ? () => setUpdating(false) : () => close(false)}
            />
          )}
          {!canManage && !endpoint && (
            <p className="text-sm text-muted-foreground">
              {ui("Connecting an endpoint requires permission to change dangerous agent settings.")}
            </p>
          )}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
        </div>
        <DrawerFooter className="items-center border-t p-4 sm:justify-between">
          <p className="text-xs text-muted-foreground">{ui("Changes here save immediately.")}</p>
          <Button variant="outline" onClick={() => close(false)}>
            {ui("Done")}
          </Button>
        </DrawerFooter>
      </DrawerContent>
    </Drawer>
  );
}
