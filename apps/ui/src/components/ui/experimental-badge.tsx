import { FlaskConical } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
  TooltipProvider,
} from "@/components/ui/tooltip";

export function ExperimentalBadge() {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <Badge
            variant="outline"
            className="ml-auto gap-1 px-1.5 py-0 text-[10px] font-semibold uppercase tracking-wider text-amber-500 border-amber-500/40 bg-amber-500/10 cursor-default select-none"
          >
            <FlaskConical className="!size-2.5" />
            Lab
          </Badge>
        </TooltipTrigger>
        <TooltipContent>
          <p>This feature is experimental. Expect changes and rough edges.</p>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
