import { forwardRef, type SVGProps } from "react";
import { cn } from "@/lib/utils";

/**
 * Agents mark: three interlocking rings drawn from the Everruns logo in the
 * Lucide stroke style, so it sits beside Lucide icons in the sidebar and
 * mastheads.
 */
export const AgentsIcon = forwardRef<SVGSVGElement, SVGProps<SVGSVGElement>>(function AgentsIcon(
  { className, ...props },
  ref,
) {
  return (
    <svg
      ref={ref}
      xmlns="http://www.w3.org/2000/svg"
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="square"
      strokeLinejoin="miter"
      aria-hidden="true"
      className={cn(className)}
      {...props}
    >
      <circle cx="12" cy="8" r="5" />
      <circle cx="7.5" cy="15.5" r="5" />
      <circle cx="16.5" cy="15.5" r="5" />
    </svg>
  );
});
