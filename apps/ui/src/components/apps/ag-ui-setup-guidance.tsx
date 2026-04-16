import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Globe } from "lucide-react";

interface AgUiSetupGuidanceProps {
  endpointUrl: string;
  isPublished: boolean;
  anonymousEnabled: boolean;
  onConfigure?: () => void;
}

export function AgUiSetupGuidance({
  endpointUrl,
  isPublished,
  anonymousEnabled,
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
