"use client";

// Explicit signup path (frames 02b -> 02c of the onboarding arc). SSO-first;
// the email path shows inline password requirements and — when the server
// runs signup email-confirm mode — always lands on a "Check your email"
// state with copy that is identical for new and already-registered
// addresses (anti-enumeration: an existing user is told to log in inside
// the email itself, never on-screen). Without confirm mode (self-host, no
// email sender) signup keeps the instant session.

import { useState, useCallback, useEffect } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AuthShell } from "@/components/auth/auth-shell";
import { OAuthProviderIcon } from "@/components/auth/oauth-provider-icon";
import { TurnstileWidget } from "@/components/auth/turnstile-widget";
import { useAuthConfig, useRegister } from "@/hooks/use-auth";
import { getOAuthUrl } from "@/lib/api/auth";
import {
  buildLoginHref,
  consumeSignupEmail,
  getPostAuthTarget,
  isBackendNavigationPath,
  persistReturnTo,
  sanitizeReturnTo,
} from "@/lib/auth-redirect";
import { usePageTitle } from "@/hooks";
import { Check, Loader2, Mail } from "lucide-react";

const MIN_LENGTH = 12;

function providerLabel(provider: string): string {
  return provider.charAt(0).toUpperCase() + provider.slice(1);
}

export default function SignupPage() {
  usePageTitle("Create your account");
  const router = useRouter();
  const searchParams = useSearchParams();
  const { data: config, isLoading: configLoading } = useAuthConfig();
  const registerMutation = useRegister();

  // Prefilled when arriving from the login door's password screen. The value is
  // a one-time sessionStorage handoff so email PII is not written into URLs.
  const [email, setEmail] = useState(() => consumeSignupEmail());
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submittedEmail, setSubmittedEmail] = useState<string | null>(null);
  const [oauthPending, setOauthPending] = useState(false);
  const [captchaToken, setCaptchaToken] = useState<string | null>(null);

  const returnTo = sanitizeReturnTo(searchParams.get("return_to"));

  useEffect(() => {
    persistReturnTo(returnTo);
  }, [returnTo]);

  const navigateAfterAuth = (target: string) => {
    if (isBackendNavigationPath(target)) {
      window.location.assign(target);
      return;
    }
    router.push(target);
  };

  const handleCaptchaVerify = useCallback((token: string) => setCaptchaToken(token), []);
  const handleCaptchaReset = useCallback(() => setCaptchaToken(null), []);

  const lengthOk = password.length >= MIN_LENGTH;
  const digitOk = /\d/.test(password);

  const handleOAuth = (provider: string) => {
    if (oauthPending) return;
    setOauthPending(true);
    persistReturnTo(returnTo);
    window.location.assign(getOAuthUrl(provider));
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    if (!lengthOk || !digitOk) {
      setError("Your password needs at least 12 characters including a number.");
      return;
    }
    if (config?.captcha && !captchaToken) {
      setError("Please complete the verification challenge below.");
      return;
    }
    const captcha_token = captchaToken ?? undefined;
    persistReturnTo(returnTo);
    try {
      await registerMutation.mutateAsync({ email, password, captcha_token });
      if (config?.signup_email_confirm) {
        // Confirm mode: no session yet — the emailed link signs you in.
        setSubmittedEmail(email);
      } else {
        navigateAfterAuth(getPostAuthTarget(returnTo));
      }
    } catch {
      if (config?.signup_email_confirm) {
        // Even unexpected failures stay generic here; the endpoint itself is
        // enumeration-safe, so don't invent a distinguishing error.
        setSubmittedEmail(email);
      } else {
        setError("We couldn't create your account. Please try again.");
      }
    }
  };

  // Frame 02c: unconditional "check your email" landing.
  if (submittedEmail) {
    return (
      <AuthShell>
        <div className="mb-6 flex h-12 w-12 items-center justify-center border border-accent/40 bg-accent/[0.12] text-accent-foreground">
          <Mail className="icon-sharp h-[22px] w-[22px]" strokeWidth={1.8} />
        </div>
        <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Check your email
        </h1>
        <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
          If <b className="font-medium text-foreground">{submittedEmail}</b>
          {" can be registered, we've sent a confirmation link. Click it to verify"}
          {" your email and get started — the link signs you in."}
        </p>
        <p className="mt-6 text-[13px] text-muted-foreground">
          Wrong address?{" "}
          <button
            type="button"
            onClick={() => setSubmittedEmail(null)}
            className="font-medium text-primary hover:underline"
          >
            Use a different email
          </button>
        </p>
        <p className="mt-2 text-[13px] text-muted-foreground">
          Already have an account?{" "}
          <Link
            href={buildLoginHref(returnTo)}
            className="font-medium text-primary hover:underline"
          >
            Log in
          </Link>
        </p>
      </AuthShell>
    );
  }

  if (configLoading) {
    return (
      <AuthShell>
        <div className="flex justify-center py-10">
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        </div>
      </AuthShell>
    );
  }

  if (config && !config.signup_enabled) {
    return (
      <AuthShell>
        <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Signups are closed
        </h1>
        <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
          New registrations are currently disabled on this deployment.
        </p>
        <p className="mt-6 text-sm text-muted-foreground">
          <Link
            href={buildLoginHref(returnTo)}
            className="font-medium text-primary hover:underline"
          >
            Back to log in
          </Link>
        </p>
      </AuthShell>
    );
  }

  const oauthProviders = config?.oauth_providers ?? [];
  // Mirror the login door: only render the email/password form when the server
  // actually accepts password registration. Otherwise the form submits into a
  // 403 and loops on a generic error — a dead end (auth-flow dead-end audit).
  const hasPasswordAuth = config?.password_auth_enabled ?? false;

  return (
    <AuthShell>
      <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
        Create your account
      </h1>
      <p className="mt-[10px] text-sm text-muted-foreground">
        Already have one?{" "}
        <Link href={buildLoginHref(returnTo)} className="font-medium text-primary hover:underline">
          Log in
        </Link>
      </p>

      <div className="mt-7 space-y-4">
        {oauthProviders.length > 0 && (
          <div className="space-y-3">
            {oauthProviders.map((provider) => (
              <Button
                key={provider}
                variant="outline"
                className="h-12 w-full justify-center gap-[10px] font-medium"
                onClick={() => handleOAuth(provider)}
                disabled={oauthPending}
              >
                <OAuthProviderIcon provider={provider} />
                Continue with {providerLabel(provider)}
              </Button>
            ))}
          </div>
        )}

        {oauthProviders.length > 0 && hasPasswordAuth && (
          <div className="flex items-center gap-3 py-1">
            <div className="h-px flex-1 bg-border" />
            <span className="font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
              or
            </span>
            <div className="h-px flex-1 bg-border" />
          </div>
        )}

        {!hasPasswordAuth && oauthProviders.length === 0 && (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No sign-up methods are currently available on this deployment.
          </p>
        )}

        {hasPasswordAuth && (
          <form onSubmit={handleSubmit} className="space-y-4">
            {error && (
              <div role="alert" className="border bg-muted/60 p-3 text-sm text-foreground">
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
            <div className="space-y-2">
              <Label htmlFor="password">Password</Label>
              <Input
                id="password"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                autoComplete="new-password"
                // No native `minLength`: it would block form submission before
                // handleSubmit runs, leaving the calm inline policy error
                // unreachable for short-but-nonempty passwords (EVE-719). Length
                // and digit rules are enforced in handleSubmit and mirrored by the
                // live requirements list below; the API stays authoritative.
                maxLength={128}
                aria-describedby="password-requirements"
              />
              {/* Inline requirements, live-checked as the user types. */}
              <ul id="password-requirements" className="space-y-1 pt-1">
                {[
                  { ok: lengthOk, label: "At least 12 characters" },
                  { ok: digitOk, label: "At least one number" },
                ].map(({ ok, label }) => (
                  <li
                    key={label}
                    className={`flex items-center gap-2 text-[12.5px] ${
                      ok ? "text-foreground" : "text-muted-foreground"
                    }`}
                  >
                    <span
                      className={`flex h-3.5 w-3.5 items-center justify-center border ${
                        ok
                          ? "border-accent bg-accent/[0.15] text-accent-foreground"
                          : "border-border"
                      }`}
                    >
                      {ok && <Check className="icon-sharp h-2.5 w-2.5" strokeWidth={3} />}
                    </span>
                    {label}
                  </li>
                ))}
              </ul>
            </div>
            <Button type="submit" className="h-11 w-full" disabled={registerMutation.isPending}>
              {registerMutation.isPending ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Creating account...
                </>
              ) : (
                "Create account"
              )}
            </Button>
          </form>
        )}

        {hasPasswordAuth && config?.captcha && (
          <TurnstileWidget
            siteKey={config.captcha.site_key}
            onVerify={handleCaptchaVerify}
            onExpire={handleCaptchaReset}
            onError={handleCaptchaReset}
          />
        )}
      </div>
    </AuthShell>
  );
}
