"use client";

// Complete a password reset — on the shared branded AuthShell. Token errors
// (expired / invalid / used) render the same generic state with a path to
// request a fresh link; no raw server messages (TM-AUTH-019).

import { useState } from "react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AuthShell } from "@/components/auth/auth-shell";
import { useResetPassword } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { ApiError } from "@/lib/api/client";
import { Check, Loader2, TriangleAlert } from "lucide-react";

// Distinguished from human-readable errors: the expired/invalid-token state
// swaps the whole form for a request-a-new-link prompt.
type FormError = { kind: "message"; text: string } | { kind: "expired" };

export default function ResetPasswordPage() {
  usePageTitle("Reset your password");
  const searchParams = useSearchParams();
  const token = searchParams.get("token");
  const resetPasswordMutation = useResetPassword();

  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState<FormError | null>(null);
  const [success, setSuccess] = useState(false);

  // Missing token or a consumed/expired one: this link can never succeed, so
  // show the invalid-link state rather than letting the user fill out a
  // doomed form.
  if (!token || error?.kind === "expired") {
    return (
      <AuthShell>
        <div className="mb-6 flex h-12 w-12 items-center justify-center border border-destructive/30 bg-destructive/10 text-destructive">
          <TriangleAlert className="icon-sharp h-[22px] w-[22px]" strokeWidth={1.8} />
        </div>
        <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Invalid reset link
        </h1>
        <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
          This reset link is invalid or has expired. Reset links are valid for one hour and can only
          be used once.
        </p>
        <Button className="mt-7" onClick={() => window.location.assign("/forgot-password")}>
          Request a new link
        </Button>
        <p className="mt-5 text-sm text-muted-foreground">
          <Link href="/login" className="font-medium text-primary hover:underline">
            Back to sign in
          </Link>
        </p>
      </AuthShell>
    );
  }

  if (success) {
    return (
      <AuthShell>
        <div className="mb-6 flex h-12 w-12 items-center justify-center border border-accent/40 bg-accent/[0.12] text-accent-foreground">
          <Check className="icon-sharp h-[22px] w-[22px]" strokeWidth={2.2} />
        </div>
        <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
          Password updated
        </h1>
        <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
          Your password has been reset and all other sessions have been signed out. You can now sign
          in with your new password.
        </p>
        <Button className="mt-7" onClick={() => window.location.assign("/login")}>
          Continue to sign in
        </Button>
      </AuthShell>
    );
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (password !== confirmPassword) {
      setError({ kind: "message", text: "Passwords do not match" });
      return;
    }
    if (password.length < 12 || !/\d/.test(password)) {
      setError({
        kind: "message",
        text: "Your new password needs at least 12 characters including a number.",
      });
      return;
    }

    try {
      await resetPasswordMutation.mutateAsync({ token, password });
      setSuccess(true);
    } catch (err) {
      if (err instanceof ApiError && err.status === 422) {
        setError({
          kind: "message",
          text: "Your new password needs 12\u2013128 characters including a number.",
        });
      } else if (err instanceof ApiError && err.status === 400) {
        // Expired/invalid/used token — swap to the request-a-new-link state.
        setError({ kind: "expired" });
      } else {
        setError({ kind: "message", text: "Something went wrong. Please try again." });
      }
    }
  };

  return (
    <AuthShell>
      <h1 className="text-[28px] font-semibold leading-none tracking-[-0.02em]">
        Set a new password
      </h1>
      <p className="mt-[10px] text-sm text-muted-foreground">
        Choose a new password for your account.
      </p>

      <form onSubmit={handleSubmit} className="mt-6 space-y-4">
        {error?.kind === "message" && (
          <div role="alert" className="bg-destructive/10 text-destructive text-sm p-3">
            {error.text}
          </div>
        )}
        <div className="space-y-2">
          <Label htmlFor="password">New password</Label>
          <Input
            id="password"
            type="password"
            placeholder="At least 12 characters, one number"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            autoComplete="new-password"
            minLength={12}
            maxLength={128}
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="confirm-password">Confirm password</Label>
          <Input
            id="confirm-password"
            type="password"
            placeholder="Repeat your new password"
            value={confirmPassword}
            onChange={(e) => setConfirmPassword(e.target.value)}
            required
            autoComplete="new-password"
            minLength={12}
            maxLength={128}
          />
        </div>
        <Button type="submit" className="w-full" disabled={resetPasswordMutation.isPending}>
          {resetPasswordMutation.isPending ? (
            <>
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              Updating...
            </>
          ) : (
            "Update password"
          )}
        </Button>
      </form>
    </AuthShell>
  );
}
