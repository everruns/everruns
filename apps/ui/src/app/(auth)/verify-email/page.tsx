"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import Image from "next/image";
import { useSearchParams } from "next/navigation";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useVerifyEmail, useResendVerification } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { Loader2 } from "lucide-react";

type VerifyState = "verifying" | "success" | "error";

export default function VerifyEmailPage() {
  usePageTitle("Verify your email");
  const searchParams = useSearchParams();
  const token = searchParams.get("token");
  // The verification link may optionally carry the email so we can offer a
  // one-click resend; without it we fall back to directing the user to sign in.
  const email = searchParams.get("email");

  const verifyEmailMutation = useVerifyEmail();
  const resendVerificationMutation = useResendVerification();

  const [state, setState] = useState<VerifyState>(token ? "verifying" : "error");
  const [resent, setResent] = useState(false);

  // Guard against React strict-mode double invocation: verify-email consumes a
  // single-use token, so a duplicate call would fail the second attempt.
  const hasVerified = useRef(false);

  useEffect(() => {
    if (!token || hasVerified.current) return;
    hasVerified.current = true;

    verifyEmailMutation
      .mutateAsync({ token })
      .then(() => setState("success"))
      .catch(() => setState("error"));
    // verifyEmailMutation is stable for the component lifetime; we deliberately
    // run this exactly once per token.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token]);

  const handleResend = async () => {
    if (!email) return;
    // Enumeration-safe endpoint: always treat as sent regardless of outcome.
    try {
      await resendVerificationMutation.mutateAsync({ email });
    } catch {
      // Intentionally ignored — never reveal whether the account exists.
    }
    setResent(true);
  };

  if (state === "verifying") {
    return (
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <div className="flex justify-center mb-4">
            <Image src="/logo.svg" alt="Everruns" width={48} height={48} />
          </div>
          <CardTitle className="text-2xl">Verifying your email</CardTitle>
          <CardDescription>This will only take a moment.</CardDescription>
        </CardHeader>
        <CardContent className="flex justify-center py-4">
          <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
        </CardContent>
      </Card>
    );
  }

  if (state === "success") {
    return (
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <div className="flex justify-center mb-4">
            <Image src="/logo.svg" alt="Everruns" width={48} height={48} />
          </div>
          <CardTitle className="text-2xl">Your email is verified</CardTitle>
          <CardDescription>Thanks for confirming your email address.</CardDescription>
        </CardHeader>
        <CardFooter className="flex justify-center">
          <Link href="/dashboard" className="text-sm text-primary hover:underline">
            Continue
          </Link>
        </CardFooter>
      </Card>
    );
  }

  // Failure state
  return (
    <Card className="w-full max-w-md">
      <CardHeader className="text-center">
        <div className="flex justify-center mb-4">
          <Image src="/logo.svg" alt="Everruns" width={48} height={48} />
        </div>
        <CardTitle className="text-2xl">Verification failed</CardTitle>
        <CardDescription>This verification link is invalid or has expired.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {resent ? (
          <div className="text-center text-sm text-muted-foreground">
            If an account exists for {email}, we&apos;ve sent a new verification link. Check your
            inbox.
          </div>
        ) : email ? (
          <Button
            className="w-full"
            onClick={handleResend}
            disabled={resendVerificationMutation.isPending}
          >
            {resendVerificationMutation.isPending ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Sending...
              </>
            ) : (
              "Resend verification email"
            )}
          </Button>
        ) : (
          <div className="text-center text-sm text-muted-foreground">
            Sign in to request a new verification email.
          </div>
        )}
      </CardContent>
      <CardFooter className="flex justify-center">
        <Link href="/login" className="text-sm text-primary hover:underline">
          Back to sign in
        </Link>
      </CardFooter>
    </Card>
  );
}
