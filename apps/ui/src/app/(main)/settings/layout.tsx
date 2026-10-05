"use client";

import Link from "next/link";
import { notFound, usePathname } from "next/navigation";
import { cn } from "@/lib/utils";
import { IconTile } from "@/components/layout/page-layout";
import { Settings as SettingsIcon } from "lucide-react";
import { useFeatureFlagsState } from "@/providers/feature-flags-provider";
import { settingsNavigationSections } from "@/lib/settings-navigation";
import { visibleNavigationSections } from "@/lib/navigation";
import { useOrg } from "@/providers/org-provider";

interface SettingsLayoutProps {
  children: React.ReactNode;
}

export default function SettingsLayout({ children }: SettingsLayoutProps) {
  const pathname = usePathname();
  const { flags, isLoading: featureFlagsLoading } = useFeatureFlagsState();
  const { hasRole } = useOrg();
  const settingsSections = visibleNavigationSections(
    settingsNavigationSections,
    flags,
    hasRole,
    process.env.NODE_ENV === "development",
  );
  const machinePaymentsEnabled = flags.machine_payments;

  if (
    !featureFlagsLoading &&
    !machinePaymentsEnabled &&
    pathname.startsWith("/settings/payments")
  ) {
    notFound();
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-4 border-b px-6 py-4">
        <IconTile icon={<SettingsIcon />} />
        <div>
          <h1 className="text-2xl font-semibold tracking-tight text-foreground">Settings</h1>
          <p className="text-sm text-muted-foreground">
            Organization defaults, providers, members, and your personal preferences.
          </p>
        </div>
      </div>
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden lg:flex-row">
        {/* Settings Sidebar */}
        <nav className="border-b bg-card p-3 lg:w-64 lg:border-b-0 lg:border-r lg:p-4 lg:overflow-y-auto">
          <div className="space-y-6">
            {settingsSections.map((section) => (
              <div key={section.label}>
                <div className="px-3 pb-2 font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
                  {section.label}
                </div>
                <div className="grid gap-1 sm:grid-cols-2 lg:block lg:space-y-1">
                  {section.items.map((item) => {
                    const isActive = pathname === item.href;
                    return (
                      <Link
                        key={item.name}
                        href={item.href}
                        prefetch={false}
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
