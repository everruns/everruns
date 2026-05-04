import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Globe } from "lucide-react";

interface AgUiSetupGuidanceProps {
  endpointUrl: string;
  isPublished: boolean;
  anonymousEnabled: boolean;
  sessionExpirationSeconds: number;
  onConfigure?: () => void;
}

export function formatSessionExpiration(seconds: number): string {
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

export function AgUiSetupGuidance({
  endpointUrl,
  isPublished,
  anonymousEnabled,
  sessionExpirationSeconds,
  onConfigure,
}: AgUiSetupGuidanceProps) {
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Badge variant={anonymousEnabled ? "default" : "secondary"}>
          {anonymousEnabled ? "Anonymous" : "Restricted"}
        </Badge>
        <span className="text-sm text-muted-foreground">
          {isPublished ? "Ready for AG-UI clients" : "Publish the app to accept requests"}
        </span>
      </div>

      <div>
        <p className="text-sm font-medium">Endpoint</p>
        <div className="mt-2 flex items-center gap-2 bg-muted p-3">
          <Globe className="h-4 w-4 shrink-0 text-muted-foreground" />
          <code className="flex-1 truncate text-sm">{endpointUrl}</code>
          <CopyButton value={endpointUrl} />
        </div>
      </div>

      <div>
        <p className="text-sm font-medium">Thread expiration</p>
        <p className="text-sm text-muted-foreground">
          {formatSessionExpiration(sessionExpirationSeconds)}
          {sessionExpirationSeconds > 0
            ? " — after this, requests reusing the same threadId are rejected with 410 Gone and the client must start a new thread"
            : " — threads can be resumed indefinitely"}
        </p>
      </div>

      <div className="space-y-1 text-sm text-muted-foreground">
        <p>Send AG-UI `RunAgentInput` JSON to this endpoint.</p>
        <p>Responses stream back as AG-UI SSE events.</p>
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
