"use client";

// Persistent nudge shown above the app shell whenever the signed-in user's
// email is not yet verified. It is the ONLY in-app path back to verification —
// login does not gate on email_verified, so an unverified user can otherwise
// roam the app with no way to satisfy the invite/verify gates they will later
// hit (auth-flow dead-end audit). Deliberately NOT dismissible: dismissing it
// would recreate the dead end it exists to close. The resend endpoint is
// enumeration-safe and only issues a token for an unverified local account.
import { useEffect, useState } from "react";
import { MailWarning, Loader2 } from "lucide-react";

import { useAuth } from "@/providers/auth-provider";
import { useResendVerification } from "@/hooks/use-auth";

// Matches the server's per-address email budget (1/minute) so the button
// re-enables roughly when the server would accept another send.
const RESEND_COOLDOWN_SECS = 60;

export function VerifyEmailBanner() {
  const { user } = useAuth();
  // Gate before the inner component so the resend mutation hook is only mounted
  // when the banner actually shows — a verified (or absent) user pays nothing.
  if (!user || user.email_verified !== false) {
    return null;
  }
  return <VerifyEmailBannerInner email={user.email} />;
}

function VerifyEmailBannerInner({ email }: { email: string }) {
  const resend = useResendVerification();
  const [cooldown, setCooldown] = useState(0);
  const [sent, setSent] = useState(false);

  useEffect(() => {
    if (cooldown <= 0) return;
    const id = setInterval(() => setCooldown((c) => (c > 0 ? c - 1 : 0)), 1000);
    return () => clearInterval(id);
  }, [cooldown]);

  const handleResend = () => {
    if (cooldown > 0 || resend.isPending) return;
    // Enumeration-safe: always treat as sent regardless of outcome.
    resend.mutate({ email }, { onSettled: () => setSent(true) });
    setCooldown(RESEND_COOLDOWN_SECS);
  };

  const label =
    cooldown > 0
      ? `Resend in 0:${String(cooldown).padStart(2, "0")}`
      : sent
        ? "Resend again"
        : "Resend verification email";

  return (
    <div
      role="status"
      className="flex h-11 flex-none items-center gap-2.5 border-b border-[hsl(var(--accent)/0.35)] bg-[hsl(var(--accent)/0.12)] px-[18px] text-primary"
    >
      <span className="inline-flex text-accent-foreground">
        <MailWarning className="h-[15px] w-[15px]" />
      </span>
      <span className="text-[13px] tracking-[-0.01em]">
        <strong className="font-semibold">Verify your email.</strong>{" "}
        {sent ? (
          <>We sent a new link to {email} — check your inbox.</>
        ) : (
          <>We sent a link to {email}. Confirm it to keep full access.</>
        )}
      </span>
      <span className="flex-1" />
      <button
        type="button"
        onClick={handleResend}
        disabled={cooldown > 0 || resend.isPending}
        className="inline-flex items-center gap-1.5 whitespace-nowrap text-[13px] font-medium text-primary hover:underline hover:underline-offset-[3px] disabled:cursor-not-allowed disabled:opacity-60 disabled:no-underline"
      >
        {resend.isPending ? <Loader2 className="h-[14px] w-[14px] animate-spin" /> : null}
        {label}
      </button>
    </div>
  );
}
