import Link from "next/link";
import { ArrowLeft, MessageCircle, SearchX } from "lucide-react";

import { buttonVariants } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface ResourceNotFoundProps {
  title: string;
  description: string;
  backHref: string;
  backLabel: string;
  resourceId?: string;
  className?: string;
}

export function ResourceNotFound({
  title,
  description,
  backHref,
  backLabel,
  resourceId,
  className,
}: ResourceNotFoundProps) {
  return (
    <div className={cn("flex min-h-[calc(100vh-2rem)] items-center justify-center p-6", className)}>
      <section
        aria-labelledby="resource-not-found-title"
        className="w-full max-w-xl border border-l-2 border-border border-l-accent bg-background/95 p-6 shadow-sm"
      >
        <div className="flex items-start gap-4">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center border border-border bg-muted text-muted-foreground">
            <SearchX className="h-5 w-5" aria-hidden="true" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-xs font-semibold uppercase tracking-[0.08em] text-muted-foreground">
              404 / Resource not found
            </p>
            <h1
              id="resource-not-found-title"
              className="mt-2 text-2xl font-semibold tracking-tight"
            >
              {title}
            </h1>
            <p className="mt-2 max-w-prose text-sm leading-6 text-muted-foreground">
              {description}
            </p>
          </div>
        </div>

        <div className="mt-6 flex flex-wrap gap-2">
          <Link href={backHref} className={buttonVariants({ variant: "default" })}>
            <ArrowLeft className="h-4 w-4" aria-hidden="true" />
            {backLabel}
          </Link>
          {backHref !== "/chats" && (
            <Link href="/chats" className={buttonVariants({ variant: "outline" })}>
              <MessageCircle className="h-4 w-4" aria-hidden="true" />
              Go to chats
            </Link>
          )}
        </div>

        {resourceId && (
          <p className="mt-5 border-t border-border pt-4 font-mono text-xs text-muted-foreground break-all">
            {resourceId}
          </p>
        )}
      </section>
    </div>
  );
}
