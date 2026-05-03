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
    <div className={cn(fullWidth ? "w-full p-6" : "container mx-auto p-6", className)}>
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
    <div className={cn("flex items-start justify-between gap-4 mb-6", className)}>
      <div className="min-w-0">
        <h1 className="text-2xl font-bold flex items-center gap-3">{title}</h1>
        {description && <p className="text-sm text-muted-foreground mt-1">{description}</p>}
      </div>
      {actions && <div className="flex items-center gap-2 shrink-0">{actions}</div>}
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
