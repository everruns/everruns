"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { AlertTriangle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

/**
 * Shown when auth bootstrap fails (config or current-user fetch errors that are
 * not a plain 401). Shared by the main and onboarding layouts so protected
 * surfaces present the same recoverable error instead of a blank screen.
 */
export function AuthUnavailableState() {
  const ui = useUiTranslations();
  return (
    <div className="flex min-h-screen items-center justify-center bg-background px-4">
      <Card className="w-full max-w-lg">
        <CardHeader className="items-center text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted">
            <AlertTriangle className="h-6 w-6 text-destructive" />
          </div>
          <CardTitle>{ui("Authentication unavailable")}</CardTitle>
          <CardDescription>
            {ui(
              "We couldn't verify your authentication state. Protected routes stay blocked until auth bootstrap succeeds.",
            )}
          </CardDescription>
        </CardHeader>
        <CardContent className="flex justify-center">
          <Button onClick={() => window.location.reload()}>{ui("Retry")}</Button>
        </CardContent>
      </Card>
    </div>
  );
}
