"use client";

import { Suspense, useEffect, useRef, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { useAuth } from "@/providers/auth-provider";
import { authKeys } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { acceptInvite } from "@/lib/api/invitations";
import { switchOrg } from "@/lib/api/users";
import { CheckCircle2, Loader2, XCircle } from "lucide-react";

type Phase = "loading" | "accepting" | "success" | "error";

function AcceptInviteInner() {
  usePageTitle("Accept invitation");
  const params = useParams<{ token: string }>();
  const token = params.token;
  const router = useRouter();
  const queryClient = useQueryClient();
  const { isAuthenticated, isLoading } = useAuth();

  const [phase, setPhase] = useState<Phase>("loading");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  // Guards the one-shot acceptance so React strict-mode/double-render can't
  // fire the POST twice.
  const startedRef = useRef(false);

  useEffect(() => {
    if (isLoading) return;

    // Unauthenticated invite links preserve the target through login and
    // resume here afterwards (see knowledge/security/authentication.md `return_to`).
    if (!isAuthenticated) {
      const returnTo = `/invite/${encodeURIComponent(token)}`;
      router.replace(`/login?return_to=${encodeURIComponent(returnTo)}`);
      return;
    }

    if (startedRef.current) return;
    startedRef.current = true;
    setPhase("accepting");

    void (async () => {
      try {
        const result = await acceptInvite(token);
        // Membership changed: refresh the user (org list) and select the org.
        try {
          await switchOrg(result.org_id);
        } catch {
          // Non-fatal: the org provider falls back to a default selection.
        }
        await queryClient.refetchQueries({ queryKey: authKeys.user() });
        setPhase("success");
      } catch (err) {
        setErrorMessage(err instanceof Error ? err.message : "We couldn't accept this invitation.");
        setPhase("error");
      }
    })();
  }, [isAuthenticated, isLoading, token, router, queryClient]);

  return (
    <div className="flex min-h-screen items-center justify-center p-4">
      <Card className="w-full max-w-md p-8 text-center">
        {(phase === "loading" || phase === "accepting") && (
          <div className="flex flex-col items-center gap-4">
            <Loader2 className="h-10 w-10 animate-spin text-muted-foreground" />
            <p className="text-muted-foreground">Accepting your invitation…</p>
          </div>
        )}

        {phase === "success" && (
          <div className="flex flex-col items-center gap-4">
            <CheckCircle2 className="h-12 w-12 text-emerald-500" />
            <div>
              <h1 className="text-lg font-semibold">You&apos;re in!</h1>
              <p className="text-sm text-muted-foreground">
                Your membership is active. Continue to your dashboard.
              </p>
            </div>
            <Button onClick={() => router.replace("/chats")}>Go to chats</Button>
          </div>
        )}

        {phase === "error" && (
          <div className="flex flex-col items-center gap-4">
            <XCircle className="h-12 w-12 text-destructive" />
            <div>
              <h1 className="text-lg font-semibold">Invitation unavailable</h1>
              <p className="text-sm text-muted-foreground">
                {errorMessage ?? "This invitation can no longer be used."}
              </p>
            </div>
            <Button variant="outline" onClick={() => router.replace("/chats")}>
              Go to chats
            </Button>
          </div>
        )}
      </Card>
    </div>
  );
}

export default function AcceptInvitePage() {
  return (
    <Suspense>
      <AcceptInviteInner />
    </Suspense>
  );
}
