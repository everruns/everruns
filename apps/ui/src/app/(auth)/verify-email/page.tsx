"use client";

// Email-verification landing page (Frame 3 of the onboarding arc) — rendered
// inside the shared AuthShell so verification feels like part of the journey
// instead of a lone card. Reached from the emailed link; the verify gate
// itself stays server-enforced.

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Button } from "@/components/ui/button";
import { AuthShell } from "@/components/auth/auth-shell";
import { useVerifyEmail, useResendVerification } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { Check, Loader2, Mail } from "lucide-react";

type VerifyState = "verifying" | "success" | "error";

const BRAND = {
  eyebrow: "Everruns",
  headline: "A verified identity is the first durable link in the chain.",
};

export default function VerifyEmailPage() {
  usePageTitle("Verify your email");
  const router = useRouter();
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
      <AuthShell brand={BRAND}>
        <div className="mb-6 flex h-12 w-12 items-center justify-center border border-accent/40 bg-accent/[0.12] text-accent-foreground">
          <Mail className="icon-sharp h-[22px] w-[22px]" strokeWidth={1.8} />
        </div>
        <h2 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Verifying your email
        </h2>
        <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
          This will only take a moment.
        </p>
        <div className="mt-7">
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        </div>
      </AuthShell>
    );
  }

  if (state === "success") {
    return (
      <AuthShell brand={BRAND}>
        <div className="mb-6 flex h-12 w-12 items-center justify-center border border-accent/40 bg-accent/[0.12] text-accent-foreground">
          <Check className="icon-sharp h-[22px] w-[22px]" strokeWidth={2.2} />
        </div>
        <h2 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Your email is verified
        </h2>
        <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
          Thanks for confirming your email address. You can pick up right where you left off.
        </p>
        <Button className="mt-7" onClick={() => router.push("/dashboard")}>
          Continue
        </Button>
      </AuthShell>
    );
  }

  // Failure state
  return (
    <AuthShell brand={BRAND}>
      <div className="mb-6 flex h-12 w-12 items-center justify-center border border-accent/40 bg-accent/[0.12] text-accent-foreground">
        <Mail className="icon-sharp h-[22px] w-[22px]" strokeWidth={1.8} />
      </div>
      <h2 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
        Verification failed
      </h2>
      <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
        This verification link is invalid or has expired.
      </p>

      <div className="mt-7 space-y-4">
        {resent ? (
          <div className="text-sm text-muted-foreground">
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
          <div className="text-sm text-muted-foreground">
            Sign in to request a new verification email.
          </div>
        )}

        <p className="text-sm text-muted-foreground">
          <Link href="/login" className="font-medium text-primary hover:underline">
            Back to sign in
          </Link>
        </p>
      </div>
    </AuthShell>
  );
}
