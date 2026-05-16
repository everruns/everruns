import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Globe } from "lucide-react";

interface FcpSetupGuidanceProps {
  endpointUrl: string;
  isPublished: boolean;
  anonymousEnabled: boolean;
  sessionExpirationSeconds: number;
  rateLimitPerMinute?: number;
  responseTimeoutSeconds?: number;
  token?: string;
  tokenConfigured?: boolean;
  hasCustomHandshake?: boolean;
  onConfigure?: () => void;
}

export function formatFcpSessionExpiration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) {
    return "Never";
  }
  const hours = seconds / 3600;
  if (hours >= 1 && Number.isInteger(hours)) {
    return `${hours} ${hours === 1 ? "hour" : "hours"}`;
  }
  if (hours >= 1) {
    return `${hours.toFixed(1)} hours`;
  }
  const minutes = seconds / 60;
  if (minutes >= 1 && Number.isInteger(minutes)) {
    return `${minutes} ${minutes === 1 ? "minute" : "minutes"}`;
  }
  return `${seconds} ${seconds === 1 ? "second" : "seconds"}`;
}

export function FcpSetupGuidance({
  endpointUrl,
  isPublished,
  anonymousEnabled,
  sessionExpirationSeconds,
  rateLimitPerMinute,
  responseTimeoutSeconds,
  token,
  tokenConfigured,
  hasCustomHandshake,
  onConfigure,
}: FcpSetupGuidanceProps) {
  const hasRateLimit = !!rateLimitPerMinute && rateLimitPerMinute > 0;
  const hasToken = !!token || !!tokenConfigured;
  const accessBadge = hasToken ? "Token Protected" : anonymousEnabled ? "Anonymous" : "Restricted";
  const timeoutSeconds =
    responseTimeoutSeconds && responseTimeoutSeconds > 0 ? responseTimeoutSeconds : 120;
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Badge variant={anonymousEnabled && !hasToken ? "default" : "secondary"}>
          {accessBadge}
        </Badge>
        <span className="text-sm text-muted-foreground">
          {isPublished ? "Ready for FCP clients" : "Publish the app to accept requests"}
        </span>
      </div>

      <div>
        <p className="text-sm font-medium">Endpoint</p>
        <div className="mt-2 flex items-center gap-2 bg-muted p-3">
          <Globe className="h-4 w-4 shrink-0 text-muted-foreground" />
          <code className="flex-1 truncate text-sm">{endpointUrl}</code>
          <CopyButton value={endpointUrl} />
        </div>
        <p className="mt-2 text-xs text-muted-foreground">
          GET this URL for the Markdown handshake. POST plain text (or JSON{" "}
          <code>&#123;&quot;message&quot;: &quot;...&quot;&#125;</code>) for a reply.
        </p>
      </div>

      <div>
        <p className="text-sm font-medium">Token</p>
        {token ? (
          <div className="mt-2 flex items-center gap-2 bg-muted p-3">
            <code className="flex-1 truncate text-sm">{token}</code>
            <CopyButton value={token} />
          </div>
        ) : hasToken ? (
          <p className="text-sm text-muted-foreground">Channel token configured</p>
        ) : anonymousEnabled ? (
          <p className="text-sm text-muted-foreground">No channel token required</p>
        ) : (
          <p className="text-sm text-muted-foreground">
            Anonymous access is off — configure a token, otherwise every request will be rejected
            with 401.
          </p>
        )}
      </div>

      <div>
        <p className="text-sm font-medium">Handshake (GET body)</p>
        <p className="text-sm text-muted-foreground">
          {hasCustomHandshake
            ? "Custom Markdown configured."
            : "Auto-generated from the app name and description."}
        </p>
      </div>

      <div>
        <p className="text-sm font-medium">Session expiration</p>
        <p className="text-sm text-muted-foreground">
          {formatFcpSessionExpiration(sessionExpirationSeconds)}
          {sessionExpirationSeconds > 0
            ? " — after this the fcp_session cookie can no longer resume the same conversation; the next request starts a fresh one."
            : " — sessions can be resumed indefinitely."}
        </p>
      </div>

      <div>
        <p className="text-sm font-medium">Rate limit</p>
        <p className="text-sm text-muted-foreground">
          {hasRateLimit
            ? `${rateLimitPerMinute} requests per minute, per client IP`
            : "No per-app cap (global API limit applies)"}
          {" — counted in an FCP-only limiter namespace."}
        </p>
      </div>

      <div>
        <p className="text-sm font-medium">Response timeout</p>
        <p className="text-sm text-muted-foreground">
          {timeoutSeconds} seconds — after which the endpoint returns 504 with the same{" "}
          <code>fcp_session</code> cookie so the client can retry.
        </p>
      </div>

      <div className="space-y-1 text-sm text-muted-foreground">
        <p>
          Body: plain UTF-8 text, or <code>application/json</code> of shape{" "}
          <code>&#123;&quot;message&quot;: &quot;...&quot;&#125;</code>. Maximum 256 KiB.
        </p>
        {hasToken && (
          <p>
            Include <code>Authorization: Bearer &lt;token&gt;</code> or{" "}
            <code>X-Everruns-FCP-Token: &lt;token&gt;</code>.
          </p>
        )}
        <p>
          Responses are <code>text/markdown</code>. Errors are also Markdown and contain the next
          step the caller should take.
        </p>
      </div>

      {onConfigure && (
        <div className="pt-1">
          <Button size="sm" variant="outline" onClick={onConfigure}>
            Configure
          </Button>
        </div>
      )}
    </div>
  );
}
