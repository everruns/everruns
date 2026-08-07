import { FlaskConical } from "lucide-react";
import { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider } from "@/components/ui/tooltip";

/**
 * Compact icon-only badge for sidebar nav items.
 * Shows a flask icon with tooltip on hover.
 */
export function ExperimentalBadge() {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="ml-auto text-amber-500 cursor-default">
            <FlaskConical className="!size-3.5" />
          </span>
        </TooltipTrigger>
        <TooltipContent>
          <p>Experimental — expect changes and rough edges.</p>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
