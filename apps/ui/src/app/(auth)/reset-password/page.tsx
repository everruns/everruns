"use client";

import { useState } from "react";
import Link from "next/link";
import Image from "next/image";
import { useSearchParams } from "next/navigation";
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
import { useResetPassword } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { ApiError } from "@/lib/api/client";
import { Loader2 } from "lucide-react";

export default function ResetPasswordPage() {
  usePageTitle("Reset your password");
  const searchParams = useSearchParams();
  const token = searchParams.get("token");
  const resetPasswordMutation = useResetPassword();

  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  // Missing token: this link can never succeed, so show the invalid-link state
  // immediately rather than letting the user fill out a doomed form.
  if (!token) {
    return (
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <div className="flex justify-center mb-4">
            <Image src="/logo.svg" alt="Everruns" width={48} height={48} />
          </div>
          <CardTitle className="text-2xl">Invalid reset link</CardTitle>
          <CardDescription>
            This reset link is invalid or has expired. Request a new one.
          </CardDescription>
        </CardHeader>
        <CardFooter className="flex justify-center">
          <Link href="/forgot-password" className="text-sm text-primary hover:underline">
            Request a new link
          </Link>
        </CardFooter>
      </Card>
    );
  }

  if (success) {
    return (
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <div className="flex justify-center mb-4">
            <Image src="/logo.svg" alt="Everruns" width={48} height={48} />
          </div>
          <CardTitle className="text-2xl">Password updated</CardTitle>
          <CardDescription>
            Your password has been reset. You can now sign in with your new password.
          </CardDescription>
        </CardHeader>
        <CardFooter className="flex justify-center">
          <Link href="/login" className="text-sm text-primary hover:underline">
            Continue to sign in
          </Link>
        </CardFooter>
      </Card>
    );
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (password !== confirmPassword) {
      setError("Passwords do not match");
      return;
    }
    if (password.length < 8) {
      setError("Password must be at least 8 characters");
      return;
    }

    try {
      await resetPasswordMutation.mutateAsync({ token, password });
      setSuccess(true);
    } catch (err) {
      if (err instanceof ApiError && err.status === 422) {
        setError("Password must be at least 8 characters");
      } else if (err instanceof ApiError && err.status === 400) {
        // Expired/invalid/used token — surface the link to request a new one.
        setError("EXPIRED");
      } else {
        setError("Something went wrong. Please try again.");
      }
    }
  };

  return (
    <Card className="w-full max-w-md">
      <CardHeader className="text-center">
        <div className="flex justify-center mb-4">
          <Image src="/logo.svg" alt="Everruns" width={48} height={48} />
        </div>
        <CardTitle className="text-2xl">Set a new password</CardTitle>
        <CardDescription>Choose a new password for your account.</CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit} className="space-y-4">
          {error === "EXPIRED" ? (
            <div className="bg-destructive/10 text-destructive text-sm p-3">
              This reset link is invalid or has expired.{" "}
              <Link href="/forgot-password" className="underline">
                Request a new one.
              </Link>
            </div>
          ) : (
            error && <div className="bg-destructive/10 text-destructive text-sm p-3">{error}</div>
          )}
          <div className="space-y-2">
            <Label htmlFor="password">New password</Label>
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
            <Label htmlFor="confirm-password">Confirm password</Label>
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
          <Button type="submit" className="w-full" disabled={resetPasswordMutation.isPending}>
            {resetPasswordMutation.isPending ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Updating...
              </>
            ) : (
              "Reset password"
            )}
          </Button>
        </form>
      </CardContent>
      <CardFooter className="flex justify-center">
        <Link href="/login" className="text-sm text-primary hover:underline">
          Back to sign in
        </Link>
      </CardFooter>
    </Card>
  );
}
