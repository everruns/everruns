import { Suspense } from "react";
import { Loader2 } from "lucide-react";

// Auth pages layout - centered, no sidebar. Sized to fit the wider two-panel
// auth shell (Frame 2); the textured background stays via bg-brand-dots.
// Suspense boundary required for useSearchParams() in login/register pages.
export default function AuthLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen flex items-center justify-center bg-background bg-brand-dots p-4 sm:p-6 lg:p-8">
      <Suspense fallback={<Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />}>
        {children}
      </Suspense>
    </div>
  );
}
