// Auth pages layout - centered, no sidebar
export default function AuthLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="min-h-screen flex items-center justify-center bg-background bg-brand-dots p-4">
      {children}
    </div>
  );
}
