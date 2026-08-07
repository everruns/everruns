// Reusable layout primitives for standard app pages.
// PageShell provides container padding (max-width by default; opt into full-width
// for dense tool pages). PageHeader renders the title/description/actions row.
// PageBody is a thin wrapper that vertically spaces content sections.

import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

interface PageShellProps {
  children: ReactNode;
  fullWidth?: boolean;
  className?: string;
}

export function PageShell({ children, fullWidth = false, className }: PageShellProps) {
  return (
    <div
      className={cn(
        fullWidth ? "w-full" : "container mx-auto",
        "min-w-0 max-w-full p-4 sm:p-6",
        className,
      )}
    >
      {children}
    </div>
  );
}

interface PageHeaderProps {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  className?: string;
}

export function PageHeader({ title, description, actions, className }: PageHeaderProps) {
  return (
    <div
      className={cn(
        "mb-6 flex min-w-0 flex-col items-start justify-between gap-4 sm:flex-row",
        className,
      )}
    >
      <div className="min-w-0">
        <h1 className="flex min-w-0 flex-wrap items-center gap-3 break-words text-2xl font-bold">
          {title}
        </h1>
        {description && (
          <p className="mt-1 break-words text-sm text-muted-foreground">{description}</p>
        )}
      </div>
      {actions && (
        <div className="flex max-w-full flex-wrap items-center gap-2 sm:shrink-0">{actions}</div>
      )}
    </div>
  );
}

interface PageBodyProps {
  children: ReactNode;
  className?: string;
}

export function PageBody({ children, className }: PageBodyProps) {
  return <div className={cn("space-y-8", className)}>{children}</div>;
}
