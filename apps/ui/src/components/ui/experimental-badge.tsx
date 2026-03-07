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

/**
 * Page-level experimental badge — small Caveat-font label
 * with a subtle hand-drawn circle. Sits inline next to page titles.
 */
export function ExperimentalPageBadge() {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="experimental-page-badge">experimental</span>
        </TooltipTrigger>
        <TooltipContent>
          <p>This feature is experimental. Expect changes and rough edges.</p>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
