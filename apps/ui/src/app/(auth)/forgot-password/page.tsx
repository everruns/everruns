"use client";

import { useState } from "react";
import Link from "next/link";
import Image from "next/image";
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
import { useForgotPassword } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { Loader2 } from "lucide-react";

export default function ForgotPasswordPage() {
  usePageTitle("Reset your password");
  const forgotPasswordMutation = useForgotPassword();

  const [email, setEmail] = useState("");
  const [submitted, setSubmitted] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    // The endpoint is enumeration-safe and always returns success, so we show
    // the same generic confirmation regardless of the result (including errors
    // such as transient network failures) to avoid leaking account existence.
    try {
      await forgotPasswordMutation.mutateAsync({ email });
    } catch {
      // Intentionally ignored — never reveal whether the account exists.
    }
    setSubmitted(true);
  };

  if (submitted) {
    return (
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <div className="flex justify-center mb-4">
            <Image src="/logo.svg" alt="Everruns" width={48} height={48} />
          </div>
          <CardTitle className="text-2xl">Check your inbox</CardTitle>
          <CardDescription>
            If an account exists for {email}, we&apos;ve sent a password reset link. Check your
            inbox.
          </CardDescription>
        </CardHeader>
        <CardFooter className="flex justify-center">
          <Link href="/login" className="text-sm text-primary hover:underline">
            Back to sign in
          </Link>
        </CardFooter>
      </Card>
    );
  }

  return (
    <Card className="w-full max-w-md">
      <CardHeader className="text-center">
        <div className="flex justify-center mb-4">
          <Image src="/logo.svg" alt="Everruns" width={48} height={48} />
        </div>
        <CardTitle className="text-2xl">Reset your password</CardTitle>
        <CardDescription>
          Enter your email and we&apos;ll send you a link to reset your password.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit} className="space-y-4">
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
      </CardContent>
      <CardFooter className="flex justify-center">
        <Link href="/login" className="text-sm text-primary hover:underline">
          Back to sign in
        </Link>
      </CardFooter>
    </Card>
  );
}
