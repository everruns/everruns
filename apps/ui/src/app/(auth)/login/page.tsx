"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import Image from "next/image";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { useAuthConfig, useLogin } from "@/hooks/use-auth";
import { getOAuthUrl } from "@/lib/api/auth";
import { Loader2 } from "lucide-react";

// OAuth provider icons/names
const oauthProviders: Record<string, { name: string; icon: string }> = {
  google: { name: "Google", icon: "/icons/google.svg" },
  github: { name: "GitHub", icon: "/icons/github.svg" },
};

export default function LoginPage() {
  const router = useRouter();
  const { data: config, isLoading: configLoading } = useAuthConfig();
  const loginMutation = useLogin();

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  // Redirect to dashboard if auth is not required
  useEffect(() => {
    if (config && config.mode === "none") {
      router.replace("/dashboard");
    }
  }, [config, router]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    try {
      await loginMutation.mutateAsync({ email, password });
      router.push("/dashboard");
    } catch (err) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError("Login failed. Please try again.");
      }
    }
  };

  const handleOAuthLogin = (provider: string) => {
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
    <Card className="w-full max-w-md">
      <CardHeader className="text-center">
        <div className="flex justify-center mb-4">
          <Image src="/logo.svg" alt="Everruns" width={48} height={48} />
        </div>
        <CardTitle className="text-2xl">Welcome back</CardTitle>
        <CardDescription>Sign in to your Everruns account</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* OAuth Buttons */}
        {hasOAuthProviders && (
          <div className="space-y-2">
            {config?.oauth_providers.map((provider) => {
              const providerInfo = oauthProviders[provider];
              return (
                <Button
                  key={provider}
                  variant="outline"
                  className="w-full"
                  onClick={() => handleOAuthLogin(provider)}
                >
                  {providerInfo?.icon && (
                    <Image
                      src={providerInfo.icon}
                      alt={providerInfo.name}
                      width={20}
                      height={20}
                      className="mr-2"
                    />
                  )}
                  Continue with {providerInfo?.name ?? provider}
                </Button>
              );
            })}
          </div>
        )}

        {/* Separator between OAuth and password */}
        {hasOAuthProviders && hasPasswordAuth && (
          <div className="relative">
            <div className="absolute inset-0 flex items-center">
              <Separator className="w-full" />
            </div>
            <div className="relative flex justify-center text-xs uppercase">
              <span className="bg-card px-2 text-muted-foreground">
                Or continue with
              </span>
            </div>
          </div>
        )}

        {/* Email/Password Form */}
        {hasPasswordAuth && (
          <form onSubmit={handleSubmit} className="space-y-4">
            {error && (
              <div className="bg-destructive/10 text-destructive text-sm p-3 rounded-md">
                {error}
              </div>
            )}
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
                placeholder="Enter your password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                autoComplete="current-password"
              />
            </div>
            <Button
              type="submit"
              className="w-full"
              disabled={loginMutation.isPending}
            >
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
      </CardContent>

      {/* Sign up link */}
      {canSignup && hasPasswordAuth && (
        <CardFooter className="flex justify-center">
          <p className="text-sm text-muted-foreground">
            Don&apos;t have an account?{" "}
            <Link href="/register" className="text-primary hover:underline">
              Sign up
            </Link>
          </p>
        </CardFooter>
      )}
    </Card>
  );
}
