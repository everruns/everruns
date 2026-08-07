import type { ReactNode } from "react";
import { CopyButton } from "@/components/ui/copy-button";
import { cn } from "@/lib/utils";

interface EntityIdentityProps {
  /** Human-readable identity rendered as the primary content. */
  children: ReactNode;
  /** Exact API-facing ID copied to the clipboard, including any namespace prefix. */
  value: string;
  /** Keep the readable label to one line. Disable for page-level headings. */
  truncate?: boolean;
  className?: string;
  labelClassName?: string;
}

/**
 * Canonical entity identity line: a readable name followed immediately by a
 * compact, accessible exact-ID copy action. The ID stays out of the layout.
 */
export function EntityIdentity({
  children,
  value,
  truncate = true,
  className,
  labelClassName,
}: EntityIdentityProps) {
  if (!truncate) {
    return (
      <span
        data-slot="entity-identity"
        className={cn("min-w-0 max-w-full align-middle", className)}
      >
        <span
          data-slot="entity-identity-label"
          className={cn("min-w-0 break-words", labelClassName)}
        >
          {children}
        </span>
        <CopyButton
          value={value}
          label={`Copy ID: ${value}`}
          kind="id"
          className="ml-1.5 align-middle"
        />
      </span>
    );
  }

  return (
    <span
      data-slot="entity-identity"
      className={cn("inline-flex min-w-0 max-w-full items-center gap-1.5 align-middle", className)}
    >
      <span data-slot="entity-identity-label" className={cn("min-w-0 truncate", labelClassName)}>
        {children}
      </span>
      <CopyButton value={value} label={`Copy ID: ${value}`} kind="id" />
    </span>
  );
}
