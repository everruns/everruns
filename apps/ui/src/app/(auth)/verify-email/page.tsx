"use client";

// Email-verification landing page (Frame 3 of the onboarding arc) — rendered
// inside the shared AuthShell so verification feels like part of the journey
// instead of a lone card. Reached from the emailed link; the verify gate
// itself stays server-enforced.

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AuthShell } from "@/components/auth/auth-shell";
import { useVerifyEmail, useResendVerification } from "@/hooks/use-auth";
import { consumeReturnTo, isBackendNavigationPath } from "@/lib/auth-redirect";
import { usePageTitle } from "@/hooks";
import { Check, Loader2, Mail } from "lucide-react";

type VerifyState = "verifying" | "success" | "error" | "pending";

// Matches the server's per-address email budget (1/minute) so the button
// unlocks roughly when the server would accept another send.
const RESEND_COOLDOWN_SECS = 60;

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

  // No token + an email means "we just sent you a link" (post-signup nudge);
  // no token and no email is a dead link.
  const [state, setState] = useState<VerifyState>(
    token ? "verifying" : email ? "pending" : "error",
  );
  const [resent, setResent] = useState(false);
  const [cooldown, setCooldown] = useState(0);
  // When the link carried no email (dead-link landing), let the user supply one
  // so they can self-serve a fresh link instead of being told to "sign in to
  // request" a screen that does not exist (auth-flow dead-end audit).
  const [manualEmail, setManualEmail] = useState("");
  const effectiveEmail = email ?? (manualEmail.trim() || null);

  // Tick the resend cooldown down once armed.
  useEffect(() => {
    if (cooldown <= 0) return;
    const id = setInterval(() => setCooldown((c) => (c > 0 ? c - 1 : 0)), 1000);
    return () => clearInterval(id);
  }, [cooldown]);

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
    if (!effectiveEmail || cooldown > 0) return;
    // Enumeration-safe endpoint: always treat as sent regardless of outcome.
    try {
      await resendVerificationMutation.mutateAsync({ email: effectiveEmail });
    } catch {
      // Intentionally ignored — never reveal whether the account exists.
    }
    setResent(true);
    setCooldown(RESEND_COOLDOWN_SECS);
  };

  const resendLabel =
    cooldown > 0
      ? `Resend link · 0:${String(cooldown).padStart(2, "0")}`
      : resent
        ? "Resend link again"
        : "Resend verification email";

  if (state === "pending") {
    return (
      <AuthShell brand={BRAND}>
        <div className="mb-6 flex h-12 w-12 items-center justify-center border border-accent/40 bg-accent/[0.12] text-accent-foreground">
          <Mail className="icon-sharp h-[22px] w-[22px]" strokeWidth={1.8} />
        </div>
        <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Verify your email
        </h1>
        <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
          We sent a verification link to <b className="font-medium text-foreground">{email}</b>.
          Click it to continue — this keeps your account secure and stays enforced on our side.
        </p>
        <div className="mt-7 flex flex-wrap gap-3">
          <Button
            variant="outline"
            onClick={handleResend}
            disabled={cooldown > 0 || resendVerificationMutation.isPending}
          >
            {resendVerificationMutation.isPending ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Sending...
              </>
            ) : (
              resendLabel
            )}
          </Button>
        </div>
        <p className="mt-6 text-[13px] text-muted-foreground">
          Wrong address?{" "}
          <Link href="/login" className="font-medium text-primary hover:underline">
            Use a different email
          </Link>
        </p>
      </AuthShell>
    );
  }

  if (state === "verifying") {
    return (
      <AuthShell brand={BRAND}>
        <div className="mb-6 flex h-12 w-12 items-center justify-center border border-accent/40 bg-accent/[0.12] text-accent-foreground">
          <Mail className="icon-sharp h-[22px] w-[22px]" strokeWidth={1.8} />
        </div>
        <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Verifying your email
        </h1>
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
        <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Your email is verified
        </h1>
        <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
          Thanks for confirming your email address. You can pick up right where you left off.
        </p>
        {/* Resume a stored invite/deep-link target when present; otherwise fall
            back to dashboard (which may continue onboarding for zero-org users). */}
        <Button
          className="mt-7"
          onClick={() => {
            const target = consumeReturnTo() || "/chats";
            if (isBackendNavigationPath(target)) {
              window.location.assign(target);
              return;
            }
            router.push(target);
          }}
        >
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
      <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
        Verification failed
      </h1>
      <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
        This verification link is invalid or has expired.
      </p>

      <div className="mt-7 space-y-4">
        {resent ? (
          <div className="text-sm text-muted-foreground">
            If an account exists for {effectiveEmail}, we&apos;ve sent a new verification link.
            Check your inbox.
          </div>
        ) : email ? (
          <Button
            className="w-full"
            onClick={handleResend}
            disabled={cooldown > 0 || resendVerificationMutation.isPending}
          >
            {resendVerificationMutation.isPending ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Sending...
              </>
            ) : (
              resendLabel
            )}
          </Button>
        ) : (
          // Dead-link landing (no email in the URL): let the user request a
          // fresh link in place rather than dead-ending on "sign in to request".
          <form
            className="space-y-3"
            onSubmit={(e) => {
              e.preventDefault();
              void handleResend();
            }}
          >
            <div className="space-y-2">
              <Label htmlFor="resend-email">Enter your email to get a new link</Label>
              <Input
                id="resend-email"
                type="email"
                placeholder="you@company.com"
                value={manualEmail}
                onChange={(e) => setManualEmail(e.target.value)}
                required
                autoComplete="email"
              />
            </div>
            <Button
              type="submit"
              className="w-full"
              disabled={!effectiveEmail || cooldown > 0 || resendVerificationMutation.isPending}
            >
              {resendVerificationMutation.isPending ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Sending...
                </>
              ) : (
                resendLabel
              )}
            </Button>
          </form>
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
