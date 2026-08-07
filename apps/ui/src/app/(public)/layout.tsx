// Isolated layout for public-facing surfaces (e.g. Public Chat).
//
// Deliberately minimal: no sidebar, no command palette, and no auth guard. The
// console's auth/org providers are mounted by the root layout but are unused
// here — public surfaces authenticate their own visitors (anonymous, or via the
// channel's own sign-in) and never depend on an Everruns user session. This
// keeps the surface portable for a future move to a dedicated domain.
// See knowledge/integrations/public-chat.md.

export default function PublicLayout({ children }: { children: React.ReactNode }) {
  return <div className="min-h-screen bg-background">{children}</div>;
}
