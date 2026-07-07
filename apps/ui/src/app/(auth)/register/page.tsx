"use client";

// Legacy alias: /register forwards to the explicit /signup path (old links
// and bookmarks), preserving return_to.

import { useEffect } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { sanitizeReturnTo } from "@/lib/auth-redirect";
import { Loader2 } from "lucide-react";

export default function RegisterPage() {
  const router = useRouter();
  const searchParams = useSearchParams();

  useEffect(() => {
    const returnTo = sanitizeReturnTo(searchParams.get("return_to"));
    router.replace(returnTo ? `/signup?return_to=${encodeURIComponent(returnTo)}` : "/signup");
  }, [router, searchParams]);

  return (
    <div className="flex items-center justify-center">
      <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
    </div>
  );
}
