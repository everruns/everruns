"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AuthShell } from "@/components/auth/auth-shell";
import { OAuthProviderIcon } from "@/components/auth/oauth-provider-icon";
import { useAuthConfig, useRegister } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { getOAuthUrl } from "@/lib/api/auth";
import { sanitizeReturnTo } from "@/lib/auth-redirect";
import { Loader2 } from "lucide-react";

// Session storage key for preserving return_to across OAuth redirects
// (must match the login page so the flow round-trips through either entry).
const RETURN_TO_KEY = "everruns_return_to";

// OAuth provider display names (logos come from OAuthProviderIcon)
const oauthProviders: Record<string, { name: string }> = {
  google: { name: "Google" },
  github: { name: "GitHub" },
};

export default function RegisterPage() {
  usePageTitle("Sign up");
  const router = useRouter();
  const searchParams = useSearchParams();
  const { data: config, isLoading: configLoading } = useAuthConfig();
  const registerMutation = useRegister();

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  // Sanitize to a safe relative path — prevents open-redirect into
  // attacker-controlled origins (see specs/authentication.md).
  const returnTo = sanitizeReturnTo(searchParams.get("return_to"));

  // Redirect if auth is not required or signup is disabled
  useEffect(() => {
    if (config) {
      if (config.mode === "none") {
        router.replace("/dashboard");
      } else if (!config.signup_enabled) {
        router.replace("/login");
      }
    }
  }, [config, router]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    // Minimal signup: only the documented 8-char minimum is enforced here; the
    // server is the trust boundary and derives a display name from the email.
    if (password.length < 8) {
      setError("Password must be at least 8 characters");
      return;
    }

    try {
      await registerMutation.mutateAsync({ email, password });
      router.push(returnTo || "/dashboard");
    } catch (err) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError("Registration failed. Please try again.");
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

  // If signup is not available, show nothing (will redirect)
  if (config?.mode === "none" || !config?.signup_enabled) {
    return null;
  }

  const hasPasswordAuth = config?.password_auth_enabled ?? false;
  const hasOAuthProviders = (config?.oauth_providers?.length ?? 0) > 0;

  return (
    <AuthShell
      brand={{
        eyebrow: "Everruns",
        headline: "Ship AI agents that don't fall over.",
      }}
    >
      <h2 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
        Create an account
      </h2>
      <p className="mt-[10px] text-sm text-muted-foreground">Get started with Everruns</p>

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
                  <OAuthProviderIcon provider={provider} />
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
              <Label htmlFor="password">Password</Label>
              <Input
                id="password"
                type="password"
                placeholder="At least 8 characters"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                autoComplete="new-password"
                minLength={8}
              />
            </div>
            <Button type="submit" className="w-full" disabled={registerMutation.isPending}>
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

        {/* No auth methods available */}
        {!hasPasswordAuth && !hasOAuthProviders && (
          <div className="text-center text-muted-foreground py-4">
            No authentication methods are currently configured.
          </div>
        )}
      </div>

      <p className="mt-[18px] text-sm text-muted-foreground">
        Already have an account?{" "}
        <Link
          href={returnTo ? `/login?return_to=${encodeURIComponent(returnTo)}` : "/login"}
          className="font-medium text-primary hover:underline"
        >
          Sign in
        </Link>
      </p>
    </AuthShell>
  );
}
