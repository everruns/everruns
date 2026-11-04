"use client";
import {
  createContext,
  useContext,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { ConfirmDialog } from "./confirm-dialog";
import { useUiTranslations } from "@/providers/locale-provider";

const ConfirmationContext = createContext<((message: string) => Promise<boolean>) | null>(null);

// Mount inside the page Chat context: Dialog's manual-interaction guard must apply.
// THREAT[TM-WEB-016]: cancel on scope changes without remounting page drafts.
export function ConfirmationProvider({
  children,
  scope = "page",
}: {
  children: ReactNode;
  scope?: string;
}) {
  const ui = useUiTranslations();
  const [requestState, setRequestState] = useState<{ scope: string; message: string } | null>(null);
  // Forget old prompts as well as cancelling their promises: returning to a route
  // must never reopen a discarded confirmation. This does not remount children.
  if (requestState && requestState.scope !== scope) setRequestState(null);
  const message = requestState?.scope === scope ? requestState.message : null;
  const resolver = useRef<((confirmed: boolean) => void) | null>(null);
  function finish(confirmed: boolean) {
    resolver.current?.(confirmed);
    resolver.current = null;
    setRequestState(null);
  }
  useLayoutEffect(
    () => () => {
      resolver.current?.(false);
      resolver.current = null;
    },
    [scope],
  );
  function request(message: string) {
    resolver.current?.(false);
    setRequestState({ scope, message });
    return new Promise<boolean>((resolve) => {
      resolver.current = resolve;
    });
  }
  return (
    <ConfirmationContext value={request}>
      {children}
      <ConfirmDialog
        open={message !== null}
        onOpenChange={(open) => {
          if (!open) finish(false);
        }}
        title={ui("Confirm action")}
        description={message}
        confirmLabel={ui("Continue")}
        onConfirm={async () => finish(true)}
      />
    </ConfirmationContext>
  );
}

export function useConfirmation() {
  const request = useContext(ConfirmationContext);
  if (!request) throw new Error("useConfirmation requires ConfirmationProvider");
  return request;
}
