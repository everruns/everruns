import { forwardRef, type SVGProps } from "react";
import { cn } from "@/lib/utils";

/**
 * Custom Twitter icon, drop-in replacement for the `Twitter` icon that
 * lucide-react removed in 1.0 (brand-icon cleanup). Path matches the lucide
 * v0 Twitter mark for visual parity at existing call sites.
 */
export const TwitterIcon = forwardRef<SVGSVGElement, SVGProps<SVGSVGElement>>(function TwitterIcon(
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
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={cn(className)}
      {...props}
    >
      <path d="M22 4s-.7 2.1-2 3.4c1.6 10-9.4 17.3-18 11.6 2.2.1 4.4-.6 6-2C3 15.5.5 9.6 3 5c2.2 2.6 5.6 4.1 9 4-.9-4.2 4-6.6 7-3.8 1.1 0 3-1.2 3-1.2z" />
    </svg>
  );
});
