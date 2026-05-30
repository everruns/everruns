"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/lib/utils";
import { Server, Key, Users, Building2, Cable, User, WalletCards, FlaskConical } from "lucide-react";

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
    label: "Organization",
    items: [
      {
        name: "Organization",
        href: "/settings/organization",
        icon: Building2,
        description: "Manage organization defaults and memberships",
      },
      {
        name: "LLM Providers",
        href: "/settings/providers",
        icon: Server,
        description: "Manage LLM providers",
      },
      {
        name: "Members",
        href: "/settings/members",
        icon: Users,
        description: "View and manage team members",
      },
      {
        name: "Features",
        href: "/settings/features",
        icon: FlaskConical,
        description: "Enable optional and experimental capabilities",
      },
      {
        name: "Payments",
        href: "/settings/payments",
        icon: WalletCards,
        description: "Manage payment wallets and spend policies",
      },
    ],
  },
  {
    label: "Personal",
    items: [
      {
        name: "Profile",
        href: "/settings/profile",
        icon: User,
        description: "Manage your profile",
      },
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
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b px-6 py-4">
        <h1 className="text-2xl font-bold">Settings</h1>
      </div>
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden lg:flex-row">
        {/* Settings Sidebar */}
        <nav className="border-b bg-card p-3 lg:w-64 lg:border-b-0 lg:border-r lg:p-4 lg:overflow-y-auto">
          <div className="space-y-6">
            {settingsSections.map((section) => (
              <div key={section.label}>
                <div className="px-3 pb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground/70">
                  {section.label}
                </div>
                <div className="grid gap-1 sm:grid-cols-2 lg:block lg:space-y-1">
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
        <div className="min-w-0 flex-1 overflow-y-auto p-4 sm:p-6">{children}</div>
      </div>
    </div>
  );
}
