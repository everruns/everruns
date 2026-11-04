"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useEffect } from "react";
import { Button } from "@/components/ui/button";
import { usePageTitle } from "@/hooks";

export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  const ui = useUiTranslations();
  usePageTitle(ui("Something went wrong"));
  useEffect(() => {
    console.error("Unhandled error:", error);
  }, [error]);

  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-background bg-brand-dots px-4">
      <div className="w-full max-w-md border bg-card p-10 text-center">
        <p className="font-mono text-5xl font-semibold text-muted-foreground">500</p>
        <h1 className="mt-4 text-2xl font-semibold tracking-tight">{ui("Something went wrong")}</h1>
        <p className="mt-2 text-muted-foreground">
          {ui("An unexpected error occurred. Please try again.")}
        </p>
        <div className="mt-6">
          <Button onClick={reset}>{ui("Try again")}</Button>
        </div>
      </div>
    </div>
  );
}
