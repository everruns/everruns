"use client";

// Unified auth entry (Frames 1–2 of the onboarding arc): one door for both
// log in and sign up, SSO-primary. Phase "email" shows the OAuth buttons and
// an email field; phase "password" asks for the password with the email
// locked in. Login vs signup is the same action — authenticate, and when the
// account doesn't exist yet (401) and signup is open, create it with the same
// credentials. All failure copy stays generic (TM-AUTH-014/019): the door
// never reveals whether an email already has an account.

import { useState, useEffect, useMemo, useRef } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AuthShell } from "@/components/auth/auth-shell";
import { OAuthProviderIcon } from "@/components/auth/oauth-provider-icon";
import { useAuthConfig, useLogin } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { ApiError } from "@/lib/api/client";
import { getOAuthUrl, login } from "@/lib/api/auth";
import {
  isBackendNavigationPath,
  sanitizeReturnTo,
  buildSignupHref,
  RETURN_TO_STORAGE_KEY,
  persistReturnTo,
  persistSignupEmail,
} from "@/lib/auth-redirect";
import { ArrowLeft, ArrowRight, Loader2 } from "lucide-react";

// OAuth provider display names (logos come from OAuthProviderIcon)
const oauthProviders: Record<string, { name: string }> = {
  google: { name: "Google" },
  github: { name: "GitHub" },
};

// New accounts require 8+ characters (server-enforced); used to decide
// whether a failed login is worth retrying as a signup.

type Phase = "email" | "password";

// Friendly copy for the coarse categories the OAuth callback redirects with
// (knowledge/security/authentication.md § OAuth Callback Failure UX). Unknown values fall
// back to the generic line so a bad query param can't inject copy.
const OAUTH_ERROR_COPY: Record<string, string> = {
  oauth_cancelled: "Sign-in was cancelled. You can try again anytime.",
  oauth_not_permitted:
    "That account can't be used to sign in here. Try a different account or continue with email.",
  oauth_failed: "Sign-in with the provider didn't complete. Try again or continue with email.",
  // Permanent, not transient: the verified email already has an account bound to
  // another sign-in method. Name the way through instead of "try again".
  oauth_account_exists:
    "This email already has an account. Sign in with the method you first used — your password below, or your original provider.",
};

export default function LoginPage() {
  usePageTitle("Log in");
  const router = useRouter();
  const searchParams = useSearchParams();
  const { data: config, isLoading: configLoading } = useAuthConfig();
  const loginMutation = useLogin();

  const [phase, setPhase] = useState<Phase>("email");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  // True when the current error is the generic credential failure — the one
  // state where the user needs an explicit way forward (password reset).
  const [showRecovery, setShowRecovery] = useState(false);
  // Disable the SSO buttons once one is clicked — the full-page redirect can
  // take a beat and double-clicks would restart the OAuth dance.
  const [oauthPending, setOauthPending] = useState(false);
  const passwordRef = useRef<HTMLInputElement>(null);
  const emailRef = useRef<HTMLInputElement>(null);

  // OAuth callback failures land back here as ?error=<category>.
  const oauthError = useMemo(() => {
    const category = searchParams.get("error");
    return category ? (OAUTH_ERROR_COPY[category] ?? OAUTH_ERROR_COPY.oauth_failed) : null;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams]);

  // Sanitize to a safe relative path — prevents open-redirect into
  // attacker-controlled origins (see knowledge/security/authentication.md).
  const returnTo = sanitizeReturnTo(searchParams.get("return_to"));

  // Redirect to dashboard if auth is not required
  useEffect(() => {
    if (config && config.mode === "none") {
      router.replace("/chats");
    }
  }, [config, router]);

  // Focus the password field when the second phase opens.
  useEffect(() => {
    if (phase === "password") {
      passwordRef.current?.focus();
    }
  }, [phase]);

  const getRedirectTarget = (): string => {
    // Check URL param first, then sessionStorage (for OAuth flow).
    // Both are sanitized so a poisoned sessionStorage can't redirect off-origin.
    const stored = sanitizeReturnTo(sessionStorage.getItem(RETURN_TO_STORAGE_KEY));
    sessionStorage.removeItem(RETURN_TO_STORAGE_KEY);
    return returnTo || stored || "/chats";
  };

  const hasPasswordAuth = config?.password_auth_enabled ?? false;
  const hasOAuthProviders = (config?.oauth_providers?.length ?? 0) > 0;
  const canSignup = config?.signup_enabled ?? false;

  const handleEmailContinue = (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setShowRecovery(false);
    setPhase("password");
  };

  const handleBack = () => {
    setPhase("email");
    setPassword("");
    setError(null);
    setShowRecovery(false);
    // Return focus to the email field so keyboard/screen-reader users are
    // not stranded on a removed element.
    setTimeout(() => emailRef.current?.focus(), 0);
  };

  const handlePasswordSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setShowRecovery(false);

    const target = getRedirectTarget();
    // For backend redirects (e.g. /oauth/authorize), call the API directly
    // and navigate immediately — don't go through the mutation's onSuccess
    // which refetches auth state and can trigger React re-renders that
    // race with the navigation.
    const backendTarget = isBackendNavigationPath(target);

    try {
      if (backendTarget) {
        await login({ email, password });
        window.location.assign(target);
        return;
      }
      await loginMutation.mutateAsync({ email, password });
      router.push(target);
      return;
    } catch (err) {
      // EVE-452 / TM-AUTH-019: never render raw server messages on a login
      // failure. Inspect the error type/status instead so non-credential
      // problems (network, 500, CSRF) get a useful operator-facing fallback
      // string while the credential-failure path still gets the fixed
      // generic message that closes the enumeration oracle.
      if (!(err instanceof ApiError && err.status === 401)) {
        setError("Login failed. Please try again.");
        return;
      }
    }

    // Login-first door: a 401 is always the calm generic failure. New users
    // take the explicit "Create an account" path (/signup) instead of a
    // silent register fallback — the fallback made wrong-password failures
    // ambiguous and forced signup semantics onto the login screen.
    setError("Email or password doesn't match. Try again or");
    setShowRecovery(true);
  };

  const handleOAuthLogin = (provider: string) => {
    if (oauthPending) return;
    setOauthPending(true);
    // Persist return_to in sessionStorage so it survives the OAuth redirect chain
    const target = returnTo || sessionStorage.getItem(RETURN_TO_STORAGE_KEY);
    if (target) {
      persistReturnTo(target);
    }
    window.location.assign(getOAuthUrl(provider));
  };

  // Show loading state while fetching config
  if (configLoading) {
    return (
      <div className="flex items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  // If auth is not required, show nothing (will redirect)
  if (config?.mode === "none") {
    return null;
  }

  const isSubmitting = loginMutation.isPending;

  // --- Phase 2: password, email locked in ---
  if (phase === "password" && hasPasswordAuth) {
    return (
      <AuthShell>
        <button
          type="button"
          onClick={handleBack}
          className="mb-5 inline-flex items-center gap-[7px] text-[13px] text-muted-foreground transition-colors hover:text-foreground"
        >
          <ArrowLeft className="icon-sharp h-[15px] w-[15px]" strokeWidth={2} />
          Back
        </button>
        <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Enter your password
        </h1>
        <p className="mt-[10px] text-sm text-muted-foreground">
          Logging in as <b className="font-medium text-foreground">{email}</b>.
        </p>

        <form onSubmit={handlePasswordSubmit} className="mt-6 space-y-4">
          {error && (
            <div role="alert" className="border bg-muted/60 p-3 text-sm text-foreground">
              {error}
              {showRecovery && (
                <>
                  {" "}
                  <Link
                    href={`/forgot-password?email=${encodeURIComponent(email)}`}
                    className="font-medium underline underline-offset-2 hover:no-underline"
                  >
                    reset your password
                  </Link>
                  .
                  {/* Enumeration-safe pointer shown to everyone (reveals nothing
                      about this email): an OAuth-only account has no password, so
                      reset silently no-ops — without this line those users hit a
                      dead end (auth-flow dead-end audit). */}
                  {hasOAuthProviders && (
                    <>
                      {" "}
                      Signed up with Google or GitHub?{" "}
                      <button
                        type="button"
                        onClick={handleBack}
                        className="font-medium underline underline-offset-2 hover:no-underline"
                      >
                        Go back
                      </button>{" "}
                      and use that instead.
                    </>
                  )}
                </>
              )}
            </div>
          )}
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <Label htmlFor="password">Password</Label>
              <Link
                href="/forgot-password"
                className="text-xs text-muted-foreground hover:text-primary hover:underline"
              >
                Forgot password?
              </Link>
            </div>
            <Input
              id="password"
              ref={passwordRef}
              type="password"
              placeholder="Enter your password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
              autoComplete="current-password"
            />
          </div>
          <Button type="submit" className="w-full" disabled={isSubmitting}>
            {isSubmitting ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Continuing...
              </>
            ) : (
              <>
                Continue
                <ArrowRight className="ml-2 h-4 w-4" />
              </>
            )}
          </Button>
        </form>

        {canSignup && (
          <p className="mt-5 text-[13px] text-muted-foreground">
            New to Everruns?{" "}
            <Link
              href={buildSignupHref(returnTo)}
              onClick={() => persistSignupEmail(email)}
              className="font-medium text-primary hover:underline"
            >
              Create an account
            </Link>
          </p>
        )}

        {hasOAuthProviders && (
          <p className="mt-5 border-t pt-5 text-[12.5px] leading-relaxed text-muted-foreground">
            Prefer Google or GitHub? Go back — it&apos;s one click and skips email verification.
          </p>
        )}
      </AuthShell>
    );
  }

  // --- Phase 1: one door, SSO-primary ---
  return (
    <AuthShell>
      <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">Log in</h1>
      <p className="mt-[10px] text-sm text-muted-foreground">
        Sign in to your Everruns console.
        {canSignup && (
          <>
            {" "}
            New to Everruns?{" "}
            <Link
              href={buildSignupHref(returnTo)}
              className="font-medium text-primary hover:underline"
            >
              Create an account
            </Link>
          </>
        )}
      </p>

      {oauthError && (
        <div role="alert" className="mt-4 border bg-muted/60 p-3 text-sm text-foreground">
          {oauthError}
        </div>
      )}

      <div className="mt-7 space-y-4">
        {/* OAuth Buttons */}
        {hasOAuthProviders && (
          <div className="space-y-[10px]">
            {config?.oauth_providers.map((provider) => {
              const providerInfo = oauthProviders[provider];
              return (
                <Button
                  key={provider}
                  variant="outline"
                  className="h-12 w-full justify-center gap-[10px] font-medium"
                  onClick={() => handleOAuthLogin(provider)}
                  disabled={oauthPending}
                >
                  <OAuthProviderIcon provider={provider} />
                  Continue with {providerInfo?.name ?? provider}
                </Button>
              );
            })}
          </div>
        )}

        {/* OR divider between OAuth and email */}
        {hasOAuthProviders && hasPasswordAuth && (
          <div className="flex items-center gap-[14px]">
            <span className="h-px flex-1 bg-border" />
            <span className="font-mono text-[11px] text-muted-foreground">OR</span>
            <span className="h-px flex-1 bg-border" />
          </div>
        )}

        {/* Email step — the password comes on the next screen */}
        {hasPasswordAuth && (
          <form onSubmit={handleEmailContinue} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input
                id="email"
                ref={emailRef}
                type="email"
                placeholder="you@company.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                required
                autoComplete="email"
              />
            </div>
            <Button type="submit" className="w-full">
              Continue with email
              <ArrowRight className="ml-2 h-4 w-4" />
            </Button>
          </form>
        )}

        {/* No auth methods available */}
        {!hasPasswordAuth && !hasOAuthProviders && (
          <div className="text-center text-muted-foreground py-4">
            No authentication methods are currently configured.
          </div>
        )}
      </div>
    </AuthShell>
  );
}
