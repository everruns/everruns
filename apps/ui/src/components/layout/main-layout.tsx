"use client";

import { MobileSidebar, Sidebar } from "./sidebar";

interface MainLayoutProps {
  children: React.ReactNode;
}

export function MainLayout({ children }: MainLayoutProps) {
  return (
    <div className="flex h-screen flex-col md:flex-row">
      <Sidebar className="hidden md:flex" />
      <MobileSidebar />
      <main className="min-w-0 flex-1 overflow-auto bg-background text-[15px]">{children}</main>
    </div>
  );
}
