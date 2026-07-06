"use client";

// Begin a password reset — on the shared branded AuthShell like the rest of
// the auth arc. Enumeration-safe: the confirmation never reveals whether an
// account exists. Renders the Turnstile challenge when the server advertises
// one (abuse-prone email-sending endpoint).

import { useState, useCallback } from "react";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AuthShell } from "@/components/auth/auth-shell";
import { TurnstileWidget } from "@/components/auth/turnstile-widget";
import { useAuthConfig, useForgotPassword } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { ArrowLeft, Loader2, Mail } from "lucide-react";

export default function ForgotPasswordPage() {
  usePageTitle("Reset your password");
  const { data: config } = useAuthConfig();
  const forgotPasswordMutation = useForgotPassword();

  const [email, setEmail] = useState("");
  const [submitted, setSubmitted] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [captchaToken, setCaptchaToken] = useState<string | null>(null);

  const handleCaptchaVerify = useCallback((token: string) => setCaptchaToken(token), []);
  const handleCaptchaReset = useCallback(() => setCaptchaToken(null), []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    if (config?.captcha && !captchaToken) {
      setError("Please complete the verification challenge below.");
      return;
    }
    // Enumeration-safe: treat every outcome as "sent". Only a captcha or
    // transport failure is worth surfacing.
    try {
      await forgotPasswordMutation.mutateAsync({
        email,
        captcha_token: captchaToken ?? undefined,
      });
    } catch {
      // Intentionally ignored — never reveal whether the account exists.
    }
    setSubmitted(true);
  };

  if (submitted) {
    return (
      <AuthShell>
        <div className="mb-6 flex h-12 w-12 items-center justify-center border border-accent/40 bg-accent/[0.12] text-accent-foreground">
          <Mail className="icon-sharp h-[22px] w-[22px]" strokeWidth={1.8} />
        </div>
        <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Check your inbox
        </h1>
        <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
          If an account exists for <b className="font-medium text-foreground">{email}</b>,
          we&apos;ve sent a password reset link. The link expires in one hour.
        </p>
        <p className="mt-7 text-sm text-muted-foreground">
          <Link href="/login" className="font-medium text-primary hover:underline">
            Back to sign in
          </Link>
        </p>
      </AuthShell>
    );
  }

  return (
    <AuthShell>
      <Link
        href="/login"
        className="mb-5 inline-flex items-center gap-[7px] text-[13px] text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft className="icon-sharp h-[15px] w-[15px]" strokeWidth={2} />
        Back to sign in
      </Link>
      <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
        Reset your password
      </h1>
      <p className="mt-[10px] text-sm text-muted-foreground">
        Enter your email and we&apos;ll send you a reset link.
      </p>

      <form onSubmit={handleSubmit} className="mt-6 space-y-4">
        {error && (
          <div role="alert" className="bg-destructive/10 text-destructive text-sm p-3">
            {error}
          </div>
        )}
        <div className="space-y-2">
          <Label htmlFor="email">Email</Label>
          <Input
            id="email"
            type="email"
            placeholder="you@company.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            autoComplete="email"
          />
        </div>
        <Button type="submit" className="w-full" disabled={forgotPasswordMutation.isPending}>
          {forgotPasswordMutation.isPending ? (
            <>
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              Sending...
            </>
          ) : (
            "Send reset link"
          )}
        </Button>
      </form>

      {config?.captcha && (
        <div className="mt-4">
          <TurnstileWidget
            siteKey={config.captcha.site_key}
            onVerify={handleCaptchaVerify}
            onExpire={handleCaptchaReset}
            onError={handleCaptchaReset}
          />
        </div>
      )}
    </AuthShell>
  );
}
