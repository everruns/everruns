"use client";

// Google sign-in for the Public Chat surface, using Google Identity Services.
// The button yields a Google ID token (JWT) which is sent to the public chat
// endpoint as a bearer token and validated server-side by the channel's
// `google_oidc` auth verifier. Only rendered when the channel configures Google
// sign-in. See specs/public-chat.md.

import { useEffect, useRef } from "react";

interface GoogleIdentity {
  accounts: {
    id: {
      initialize: (config: {
        client_id: string;
        callback: (response: { credential?: string }) => void;
        auto_select?: boolean;
      }) => void;
      renderButton: (
        el: HTMLElement,
        options: { theme?: string; size?: string; type?: string; width?: number },
      ) => void;
    };
  };
}

declare global {
  interface Window {
    google?: GoogleIdentity;
    __everrunsGsiLoading?: Promise<void>;
  }
}

const SCRIPT_SRC = "https://accounts.google.com/gsi/client";

function loadGsiScript(): Promise<void> {
  if (window.google?.accounts?.id) return Promise.resolve();
  if (window.__everrunsGsiLoading) return window.__everrunsGsiLoading;

  window.__everrunsGsiLoading = new Promise<void>((resolve, reject) => {
    const script = document.createElement("script");
    script.src = SCRIPT_SRC;
    script.async = true;
    script.defer = true;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error("Failed to load Google sign-in"));
    document.head.appendChild(script);
  });
  return window.__everrunsGsiLoading;
}

interface GoogleSignInButtonProps {
  clientId: string;
  onCredential: (idToken: string) => void;
  onError?: () => void;
}

export function GoogleSignInButton({ clientId, onCredential, onError }: GoogleSignInButtonProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    loadGsiScript()
      .then(() => {
        if (cancelled || !window.google || !containerRef.current) return;
        window.google.accounts.id.initialize({
          client_id: clientId,
          callback: (response) => {
            if (response.credential) onCredential(response.credential);
          },
        });
        window.google.accounts.id.renderButton(containerRef.current, {
          theme: "outline",
          size: "large",
          type: "standard",
        });
      })
      .catch(() => {
        if (!cancelled) onError?.();
      });
    return () => {
      cancelled = true;
    };
  }, [clientId, onCredential, onError]);

  return <div ref={containerRef} className="flex justify-center" />;
}
