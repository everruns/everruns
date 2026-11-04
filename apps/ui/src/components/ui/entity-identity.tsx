"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import type { ReactNode } from "react";
import { CopyButton } from "@/components/ui/copy-button";
import { cn } from "@/lib/utils";
import { PageViewEntity } from "@/components/chat-mode/page-view-state";

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
  const ui = useUiTranslations();
  if (!truncate) {
    return (
      <span
        data-slot="entity-identity"
        className={cn("inline-flex min-w-0 max-w-full items-center gap-1.5", className)}
      >
        <PageViewEntity id={value} name={typeof children === "string" ? children : undefined} />
        <span
          data-slot="entity-identity-label"
          className={cn("min-w-0 break-words", labelClassName)}
        >
          {children}
        </span>
        <CopyButton value={value} label={ui("Copy ID: {value0}", { value0: value })} kind="id" />
      </span>
    );
  }

  return (
    <span
      data-slot="entity-identity"
      className={cn("inline-flex min-w-0 max-w-full items-center gap-1.5 align-middle", className)}
    >
      <PageViewEntity id={value} name={typeof children === "string" ? children : undefined} />
      <span data-slot="entity-identity-label" className={cn("min-w-0 truncate", labelClassName)}>
        {children}
      </span>
      <CopyButton value={value} label={ui("Copy ID: {value0}", { value0: value })} kind="id" />
    </span>
  );
}
