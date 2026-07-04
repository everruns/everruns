"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AuthShell } from "@/components/auth/auth-shell";
import { useAuthConfig, useRegister } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { sanitizeReturnTo } from "@/lib/auth-redirect";
import { Loader2 } from "lucide-react";

export default function RegisterPage() {
  usePageTitle("Sign up");
  const router = useRouter();
  const searchParams = useSearchParams();
  const { data: config, isLoading: configLoading } = useAuthConfig();
  const registerMutation = useRegister();

  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  const returnTo = sanitizeReturnTo(searchParams.get("return_to"));

  // Redirect if auth is not required or signup is disabled
  useEffect(() => {
    if (config) {
      if (config.mode === "none") {
        router.replace("/dashboard");
      } else if (!config.signup_enabled || !config.password_auth_enabled) {
        router.replace("/login");
      }
    }
  }, [config, router]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    // Validate password match
    if (password !== confirmPassword) {
      setError("Passwords do not match");
      return;
    }

    // Validate password length
    if (password.length < 8) {
      setError("Password must be at least 8 characters");
      return;
    }

    try {
      await registerMutation.mutateAsync({ name, email, password });
      router.push(returnTo || "/dashboard");
    } catch (err) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError("Registration failed. Please try again.");
      }
    }
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
  if (config?.mode === "none" || !config?.signup_enabled || !config?.password_auth_enabled) {
    return null;
  }

  return (
    <AuthShell
      brand={{
        eyebrow: "Everruns",
        headline: "Start building agents that ever run.",
      }}
    >
      <h2 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
        Create an account
      </h2>
      <p className="mt-[10px] text-sm text-muted-foreground">Get started with Everruns</p>

      <form onSubmit={handleSubmit} className="mt-7 space-y-4">
        {error && <div className="bg-destructive/10 text-destructive text-sm p-3">{error}</div>}
        <div className="space-y-2">
          <Label htmlFor="name">Name</Label>
          <Input
            id="name"
            type="text"
            placeholder="Your name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
            autoComplete="name"
          />
        </div>
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
        <div className="space-y-2">
          <Label htmlFor="confirm-password">Confirm Password</Label>
          <Input
            id="confirm-password"
            type="password"
            placeholder="Confirm your password"
            value={confirmPassword}
            onChange={(e) => setConfirmPassword(e.target.value)}
            required
            autoComplete="new-password"
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
