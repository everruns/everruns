"use client";

/**
 * Decision: Centralize dev-page chrome so preview pages use one shell and can
 * focus on the component or transcript being demonstrated.
 */

import Link from "next/link";
import { ArrowLeft } from "lucide-react";

const isDev = process.env.NODE_ENV === "development";

interface DevPageShellProps {
  eyebrow: string;
  title: string;
  description: string;
  children: React.ReactNode;
  widthClassName?: string;
}

export function DevPageShell({
  eyebrow,
  title,
  description,
  children,
  widthClassName = "max-w-6xl",
}: DevPageShellProps) {
  if (!isDev) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-muted-foreground">404</h1>
          <p className="mt-2 text-muted-foreground">Page not found</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-background bg-brand-dots px-4 py-8">
      <div className={`mx-auto ${widthClassName}`}>
        <Link
          href="/dev"
          className="mb-6 inline-flex items-center gap-2 border border-border bg-card px-3 py-2 text-sm text-muted-foreground transition-colors hover:bg-muted/35 hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          Back to Developer Tools
        </Link>

        <div className="mb-8 max-w-2xl space-y-2">
          <p className="text-[11px] uppercase tracking-[0.3em] text-muted-foreground">{eyebrow}</p>
          <h1 className="text-3xl font-semibold tracking-tight text-foreground">{title}</h1>
          <p className="text-sm leading-6 text-muted-foreground">{description}</p>
        </div>

        {children}
      </div>
    </div>
  );
}
