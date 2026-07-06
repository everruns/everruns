"use client";

// The unified /login door ("Log in or sign up") replaced the separate
// registration screen — signup now happens through the same entry. The route
// stays for old links and bookmarks and simply forwards, preserving
// return_to.

import { useEffect } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { sanitizeReturnTo } from "@/lib/auth-redirect";
import { Loader2 } from "lucide-react";

export default function RegisterPage() {
  const router = useRouter();
  const searchParams = useSearchParams();

  useEffect(() => {
    const returnTo = sanitizeReturnTo(searchParams.get("return_to"));
    router.replace(returnTo ? `/login?return_to=${encodeURIComponent(returnTo)}` : "/login");
  }, [router, searchParams]);

  return (
    <div className="flex items-center justify-center">
      <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
    </div>
  );
}
