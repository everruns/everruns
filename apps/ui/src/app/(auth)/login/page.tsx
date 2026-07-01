"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import Image from "next/image";
import { useRouter, useSearchParams } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AuthShell } from "@/components/auth/auth-shell";
import { useAuthConfig, useLogin } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { ApiError } from "@/lib/api/client";
import { getOAuthUrl, login } from "@/lib/api/auth";
import { isBackendNavigationPath, sanitizeReturnTo } from "@/lib/auth-redirect";
import { Loader2 } from "lucide-react";

// Session storage key for preserving return_to across OAuth redirects
const RETURN_TO_KEY = "everruns_return_to";

// OAuth provider icons/names
const oauthProviders: Record<string, { name: string; icon: string }> = {
  google: { name: "Google", icon: "/icons/google.svg" },
  github: { name: "GitHub", icon: "/icons/github.svg" },
};

export default function LoginPage() {
  usePageTitle("Sign in");
  const router = useRouter();
  const searchParams = useSearchParams();
  const { data: config, isLoading: configLoading } = useAuthConfig();
  const loginMutation = useLogin();

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  // Sanitize to a safe relative path — prevents open-redirect into
  // attacker-controlled origins (see specs/authentication.md).
  const returnTo = sanitizeReturnTo(searchParams.get("return_to"));

  // Redirect to dashboard if auth is not required
  useEffect(() => {
    if (config && config.mode === "none") {
      router.replace("/dashboard");
    }
  }, [config, router]);

  const getRedirectTarget = (): string => {
    // Check URL param first, then sessionStorage (for OAuth flow).
    // Both are sanitized so a poisoned sessionStorage can't redirect off-origin.
    const stored = sanitizeReturnTo(sessionStorage.getItem(RETURN_TO_KEY));
    sessionStorage.removeItem(RETURN_TO_KEY);
    return returnTo || stored || "/dashboard";
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    try {
      const target = getRedirectTarget();
      // For backend redirects (e.g. /oauth/authorize), call login() directly
      // and navigate immediately — don't go through the mutation's onSuccess
      // which refetches auth state and can trigger React re-renders that
      // race with the navigation.
      if (isBackendNavigationPath(target)) {
        await login({ email, password });
        window.location.assign(target);
        return;
      }
      await loginMutation.mutateAsync({ email, password });
      router.push(target);
    } catch (err) {
      // EVE-452 / TM-AUTH-019: never render raw server messages on a login
      // failure. Inspect the error type/status instead so non-credential
      // problems (network, 500, CSRF) get a useful operator-facing fallback
      // string while the credential-failure path still gets the fixed
      // generic message that closes the enumeration oracle.
      if (err instanceof ApiError && err.status === 401) {
        setError("Invalid email or password.");
      } else {
        setError("Login failed. Please try again.");
      }
    }
  };

  const handleOAuthLogin = (provider: string) => {
    // Persist return_to in sessionStorage so it survives the OAuth redirect chain
    const target = returnTo || sessionStorage.getItem(RETURN_TO_KEY);
    if (target) {
      sessionStorage.setItem(RETURN_TO_KEY, target);
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

  const hasPasswordAuth = config?.password_auth_enabled ?? false;
  const hasOAuthProviders = (config?.oauth_providers?.length ?? 0) > 0;
  const canSignup = config?.signup_enabled ?? false;

  return (
    <AuthShell headerNote="Secured by Everruns">
      <h2 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">Welcome back</h2>
      <p className="mt-[10px] text-sm text-muted-foreground">
        Sign in to your Everruns account
      </p>

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
                  className="h-11 w-full justify-start gap-[10px]"
                  onClick={() => handleOAuthLogin(provider)}
                >
                  {providerInfo?.icon && (
                    <Image
                      src={providerInfo.icon}
                      alt={providerInfo.name}
                      width={18}
                      height={18}
                    />
                  )}
                  Continue with {providerInfo?.name ?? provider}
                </Button>
              );
            })}
          </div>
        )}

        {/* OR divider between OAuth and password */}
        {hasOAuthProviders && hasPasswordAuth && (
          <div className="flex items-center gap-[14px]">
            <span className="h-px flex-1 bg-border" />
            <span className="font-mono text-[11px] text-muted-foreground">OR</span>
            <span className="h-px flex-1 bg-border" />
          </div>
        )}

        {/* Email/Password Form */}
        {hasPasswordAuth && (
          <form onSubmit={handleSubmit} className="space-y-4">
            {error && <div className="bg-destructive/10 text-destructive text-sm p-3">{error}</div>}
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input
                id="email"
                type="email"
                placeholder="you@example.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                required
                autoComplete="email"
              />
            </div>
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
                type="password"
                placeholder="Enter your password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                autoComplete="current-password"
              />
            </div>
            <Button type="submit" className="w-full" disabled={loginMutation.isPending}>
              {loginMutation.isPending ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Signing in...
                </>
              ) : (
                "Sign in"
              )}
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

      {/* Sign up link */}
      {canSignup && hasPasswordAuth && (
        <p className="mt-[18px] text-sm text-muted-foreground">
          Don&apos;t have an account?{" "}
          <Link
            href={returnTo ? `/register?return_to=${encodeURIComponent(returnTo)}` : "/register"}
            className="font-medium text-primary hover:underline"
          >
            Sign up
          </Link>
        </p>
      )}
    </AuthShell>
  );
}
