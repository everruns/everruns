"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/lib/utils";
import { Header } from "@/components/layout/header";
import { Server, Key, Users, Plug, Building2, Cable } from "lucide-react";

interface NavItem {
  name: string;
  href: string;
  icon: React.ComponentType<{ className?: string }>;
  description: string;
}

interface NavSection {
  label: string;
  items: NavItem[];
}

const settingsSections: NavSection[] = [
  {
    label: "Organisation",
    items: [
      {
        name: "General",
        href: "/settings/organisation",
        icon: Building2,
        description: "Manage organisation settings",
      },
      {
        name: "LLM Providers",
        href: "/settings/providers",
        icon: Server,
        description: "Manage LLM providers and models",
      },
      {
        name: "MCP Servers",
        href: "/settings/mcp-servers",
        icon: Plug,
        description: "Manage MCP server connections",
      },
      {
        name: "Members",
        href: "/settings/members",
        icon: Users,
        description: "View and manage team members",
      },
    ],
  },
  {
    label: "Personal",
    items: [
      {
        name: "Connections",
        href: "/settings/connections",
        icon: Cable,
        description: "Connect external accounts",
      },
      {
        name: "API Keys",
        href: "/settings/api-keys",
        icon: Key,
        description: "Manage API keys for programmatic access",
      },
    ],
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
          <div className="space-y-6">
            {settingsSections.map((section) => (
              <div key={section.label}>
                <div className="px-3 pb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground/70">
                  {section.label}
                </div>
                <div className="space-y-1">
                  {section.items.map((item) => {
                    const isActive = pathname === item.href;
                    return (
                      <Link
                        key={item.name}
                        href={item.href}
                        className={cn(
                          "flex items-center gap-3 px-3 py-2 text-sm transition-colors border-l-2",
                          isActive
                            ? "bg-accent/10 text-accent-foreground border-accent"
                            : "text-muted-foreground hover:bg-muted hover:text-foreground border-transparent",
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
              </div>
            ))}
          </div>
        </nav>

        {/* Settings Content */}
        <div className="flex-1 overflow-y-auto p-6">{children}</div>
      </div>
    </>
  );
}
