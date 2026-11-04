"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Suspense, useEffect } from "react";
import { useSearchParams } from "next/navigation";
import { usePageTitle } from "@/hooks";
import { connectErrorMessage } from "@/lib/connect-error";

function ConnectionCompleteInner() {
  const ui = useUiTranslations();
  const searchParams = useSearchParams();
  // A failed connect lands here too (`status=error&connect_error=<code>`).
  const failed = searchParams.get("status") === "error";
  usePageTitle(failed ? "Connection failed" : "Connection complete");

  useEffect(() => {
    const payload = {
      type: "everruns:connection-complete",
      provider: searchParams.get("provider"),
      status: searchParams.get("status"),
      return_to: searchParams.get("return_to"),
      connect_error: searchParams.get("connect_error"),
    };
    if (window.opener) {
      window.opener.postMessage(payload, window.location.origin);
      window.close();
    }
  }, [searchParams]);

  return (
    <div className="flex min-h-screen items-center justify-center text-sm text-muted-foreground">
      {failed
        ? ui("{value0} You can close this window.", {
            value0: connectErrorMessage(searchParams.get("connect_error")),
          })
        : ui("Connection complete. You can close this window.")}
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
