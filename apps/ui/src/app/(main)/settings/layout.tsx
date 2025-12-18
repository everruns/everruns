"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/lib/utils";
import { Header } from "@/components/layout/header";
import { Server, Key, Users } from "lucide-react";

const settingsNavigation = [
  {
    name: "LLM Providers",
    href: "/settings/providers",
    icon: Server,
    description: "Manage LLM providers and models",
  },
  {
    name: "API Keys",
    href: "/settings/api-keys",
    icon: Key,
    description: "Manage API keys for programmatic access",
  },
  {
    name: "Members",
    href: "/settings/members",
    icon: Users,
    description: "Manage team members and access",
  },
];

interface SettingsLayoutProps {
  children: React.ReactNode;
}

export default function SettingsLayout({ children }: SettingsLayoutProps) {
  const pathname = usePathname();

  return (
    <>
      <Header title="Settings" />
      <div className="flex flex-1 overflow-hidden">
        {/* Settings Sidebar */}
        <nav className="w-64 border-r bg-card p-4 overflow-y-auto">
          <div className="space-y-1">
            {settingsNavigation.map((item) => {
              const isActive = pathname === item.href;
              return (
                <Link
                  key={item.name}
                  href={item.href}
                  className={cn(
                    "flex items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors",
                    isActive
                      ? "bg-primary text-primary-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                  )}
                >
                  <item.icon className="h-4 w-4" />
                  <div>
                    <div className="font-medium">{item.name}</div>
                  </div>
                </Link>
              );
            })}
          </div>
        </nav>

        {/* Settings Content */}
        <div className="flex-1 overflow-y-auto p-6">{children}</div>
      </div>
    </>
  );
}
