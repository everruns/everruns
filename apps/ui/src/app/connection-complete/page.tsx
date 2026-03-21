"use client";

import { Suspense, useEffect } from "react";
import { useSearchParams } from "next/navigation";

function ConnectionCompleteInner() {
  const searchParams = useSearchParams();

  useEffect(() => {
    const payload = {
      type: "everruns:connection-complete",
      provider: searchParams.get("provider"),
      status: searchParams.get("status"),
      return_to: searchParams.get("return_to"),
    };
    if (window.opener) {
      window.opener.postMessage(payload, window.location.origin);
      window.close();
    }
  }, [searchParams]);

  return (
    <div className="flex min-h-screen items-center justify-center text-sm text-muted-foreground">
      Connection complete. You can close this window.
    </div>
  );
}

export default function ConnectionCompletePage() {
  return (
    <Suspense>
      <ConnectionCompleteInner />
    </Suspense>
  );
}
