"use client";

import Link from "next/link";
import { useState } from "react";
import { ArrowLeft, Terminal, Check, Loader2, ChevronRight, Play, Square, Settings, Plus, Search, Bell, User, Database, Cpu, Activity, Code, Layers, GitBranch } from "lucide-react";
import { cn } from "@/lib/utils";

const isDev = process.env.NODE_ENV === "development";

// Everruns logo - 3 interlocking circles
function EverrunsLogo({ size = 24, accentColor = "hsl(180 70% 45%)" }: { size?: number; accentColor?: string }) {
  const scale = size / 32;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 512 512"
      style={{ transform: `scale(${scale})`, transformOrigin: 'center' }}
    >
      <defs>
        <linearGradient id="gTopTerminal" gradientUnits="userSpaceOnUse" x1="256" y1="94" x2="256" y2="284">
          <stop offset="0.00" stopColor={accentColor} stopOpacity="0.3" />
          <stop offset="0.70" stopColor={accentColor} stopOpacity="0.6" />
          <stop offset="1.00" stopColor={accentColor} />
        </linearGradient>
        <linearGradient id="gLeftTerminal" gradientUnits="userSpaceOnUse" x1="70" y1="374" x2="256" y2="284">
          <stop offset="0.00" stopColor={accentColor} stopOpacity="0.2" />
          <stop offset="0.70" stopColor={accentColor} stopOpacity="0.5" />
          <stop offset="1.00" stopColor={accentColor} />
        </linearGradient>
        <linearGradient id="gRightTerminal" gradientUnits="userSpaceOnUse" x1="442" y1="374" x2="256" y2="284">
          <stop offset="0.00" stopColor={accentColor} stopOpacity="0.15" />
          <stop offset="0.70" stopColor={accentColor} stopOpacity="0.4" />
          <stop offset="1.00" stopColor={accentColor} />
        </linearGradient>
      </defs>
      <g fill="none" strokeWidth="18" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="256" cy="214" r="120" stroke="url(#gTopTerminal)" />
        <circle cx="186" cy="309" r="120" stroke="url(#gLeftTerminal)" />
        <circle cx="326" cy="309" r="120" stroke="url(#gRightTerminal)" />
      </g>
    </svg>
  );
}

/**
 * Design Concept: "Terminal"
 *
 * Philosophy:
 * - Sharp corners (0 radius) for a technical, precise feel
 * - Cyan accent color inspired by terminal aesthetics
 * - High contrast, dark-first design
 * - Monospace elements for developer familiarity
 * - Minimal chrome, maximum content
 */

// Theme colors (would be CSS variables in production)
const theme = {
  // Primary accent - Cyan/Teal
  accent: "hsl(180 70% 45%)",
  accentMuted: "hsl(180 70% 45% / 0.15)",
  accentBorder: "hsl(180 70% 45% / 0.3)",

  // Backgrounds
  bgDark: "hsl(220 20% 8%)",
  bgCard: "hsl(220 15% 12%)",
  bgHover: "hsl(220 15% 15%)",
  bgMuted: "hsl(220 15% 18%)",

  // Text
  textPrimary: "hsl(0 0% 95%)",
  textSecondary: "hsl(0 0% 65%)",
  textMuted: "hsl(0 0% 45%)",

  // Borders
  border: "hsl(220 15% 20%)",
  borderSubtle: "hsl(220 15% 15%)",

  // Status colors
  success: "hsl(145 70% 45%)",
  warning: "hsl(40 90% 50%)",
  error: "hsl(0 70% 55%)",
};

// Sharp corner button
function SharpButton({
  children,
  variant = "default",
  size = "default",
  className = ""
}: {
  children: React.ReactNode;
  variant?: "default" | "accent" | "ghost" | "outline";
  size?: "default" | "sm" | "icon";
  className?: string;
}) {
  const variants = {
    default: "bg-[hsl(220_15%_18%)] text-[hsl(0_0%_95%)] hover:bg-[hsl(220_15%_22%)] border border-[hsl(220_15%_25%)]",
    accent: "bg-[hsl(180_70%_45%)] text-[hsl(220_20%_8%)] hover:bg-[hsl(180_70%_50%)] font-medium",
    ghost: "text-[hsl(0_0%_65%)] hover:text-[hsl(0_0%_95%)] hover:bg-[hsl(220_15%_15%)]",
    outline: "border border-[hsl(180_70%_45%/_0.3)] text-[hsl(180_70%_45%)] hover:bg-[hsl(180_70%_45%/_0.1)]",
  };

  const sizes = {
    default: "px-4 py-2 text-sm",
    sm: "px-3 py-1.5 text-xs",
    icon: "p-2",
  };

  return (
    <button className={cn(
      "inline-flex items-center justify-center gap-2 rounded-none transition-colors",
      variants[variant],
      sizes[size],
      className
    )}>
      {children}
    </button>
  );
}

// Sharp card
function SharpCard({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={cn(
      "rounded-none border border-[hsl(220_15%_20%)] bg-[hsl(220_15%_12%)]",
      className
    )}>
      {children}
    </div>
  );
}

// Sharp input
function SharpInput({ placeholder, className = "" }: { placeholder?: string; className?: string }) {
  return (
    <input
      type="text"
      placeholder={placeholder}
      className={cn(
        "w-full rounded-none border border-[hsl(220_15%_20%)] bg-[hsl(220_15%_8%)] px-3 py-2 text-sm",
        "text-[hsl(0_0%_95%)] placeholder:text-[hsl(0_0%_45%)]",
        "focus:outline-none focus:border-[hsl(180_70%_45%)] focus:ring-1 focus:ring-[hsl(180_70%_45%/_0.3)]",
        className
      )}
    />
  );
}

// Sharp badge
function SharpBadge({
  children,
  variant = "default"
}: {
  children: React.ReactNode;
  variant?: "default" | "success" | "warning" | "accent";
}) {
  const variants = {
    default: "bg-[hsl(220_15%_18%)] text-[hsl(0_0%_65%)] border-[hsl(220_15%_25%)]",
    success: "bg-[hsl(145_70%_45%/_0.15)] text-[hsl(145_70%_55%)] border-[hsl(145_70%_45%/_0.3)]",
    warning: "bg-[hsl(40_90%_50%/_0.15)] text-[hsl(40_90%_60%)] border-[hsl(40_90%_50%/_0.3)]",
    accent: "bg-[hsl(180_70%_45%/_0.15)] text-[hsl(180_70%_55%)] border-[hsl(180_70%_45%/_0.3)]",
  };

  return (
    <span className={cn(
      "inline-flex items-center px-2 py-0.5 text-xs font-mono rounded-none border",
      variants[variant]
    )}>
      {children}
    </span>
  );
}

// Chat message components
function TerminalUserMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] rounded-none border border-[hsl(180_70%_45%/_0.3)] bg-[hsl(180_70%_45%/_0.08)] px-3 py-2">
        <p className="text-sm text-[hsl(0_0%_95%)]">{content}</p>
      </div>
    </div>
  );
}

function TerminalAgentMessage({ content }: { content: string }) {
  return (
    <div className="flex gap-2">
      <div className="w-5 h-5 rounded-none bg-[hsl(180_70%_45%/_0.15)] border border-[hsl(180_70%_45%/_0.3)] flex items-center justify-center flex-shrink-0 mt-0.5">
        <Terminal className="w-3 h-3 text-[hsl(180_70%_45%)]" />
      </div>
      <p className="text-sm text-[hsl(0_0%_85%)]">{content}</p>
    </div>
  );
}

function TerminalToolCall({ name, status, result }: { name: string; status: "done" | "running"; result?: string }) {
  return (
    <div className="ml-7 font-mono text-xs">
      <div className="flex items-center gap-2 text-[hsl(0_0%_55%)]">
        {status === "running" ? (
          <Loader2 className="w-3 h-3 animate-spin text-[hsl(180_70%_45%)]" />
        ) : (
          <Check className="w-3 h-3 text-[hsl(145_70%_45%)]" />
        )}
        <span className="text-[hsl(180_70%_45%)]">{name}</span>
      </div>
      {result && (
        <div className="mt-1 pl-5 text-[hsl(0_0%_50%)] border-l border-[hsl(220_15%_20%)]">
          <span className="text-[hsl(0_0%_40%)]">&gt;</span> {result}
        </div>
      )}
    </div>
  );
}

// Sidebar nav item
function NavItem({ icon: Icon, label, active = false }: { icon: React.ElementType; label: string; active?: boolean }) {
  return (
    <div className={cn(
      "flex items-center gap-3 px-3 py-2 text-sm cursor-pointer transition-colors rounded-none",
      active
        ? "bg-[hsl(180_70%_45%/_0.15)] text-[hsl(180_70%_55%)] border-l-2 border-[hsl(180_70%_45%)]"
        : "text-[hsl(0_0%_55%)] hover:text-[hsl(0_0%_85%)] hover:bg-[hsl(220_15%_15%)]"
    )}>
      <Icon className="w-4 h-4" />
      <span>{label}</span>
    </div>
  );
}

// Stats card
function StatCard({ label, value, icon: Icon }: { label: string; value: string; icon: React.ElementType }) {
  return (
    <SharpCard className="p-4">
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs text-[hsl(0_0%_50%)] uppercase tracking-wide">{label}</span>
        <Icon className="w-4 h-4 text-[hsl(180_70%_45%)]" />
      </div>
      <div className="text-2xl font-mono text-[hsl(0_0%_95%)]">{value}</div>
    </SharpCard>
  );
}

// Session row
function SessionRow({ id, status, model }: { id: string; status: "running" | "completed"; model: string }) {
  return (
    <div className="flex items-center justify-between px-4 py-3 border-b border-[hsl(220_15%_15%)] hover:bg-[hsl(220_15%_14%)] cursor-pointer transition-colors">
      <div className="flex items-center gap-3">
        {status === "running" ? (
          <div className="w-2 h-2 rounded-none bg-[hsl(180_70%_45%)] animate-pulse" />
        ) : (
          <div className="w-2 h-2 rounded-none bg-[hsl(145_70%_45%)]" />
        )}
        <span className="font-mono text-sm text-[hsl(0_0%_85%)]">{id}</span>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-xs text-[hsl(0_0%_50%)]">{model}</span>
        <ChevronRight className="w-4 h-4 text-[hsl(0_0%_40%)]" />
      </div>
    </div>
  );
}

export default function DesignTerminalPage() {
  const [activeNav, setActiveNav] = useState("agents");

  if (!isDev) {
    return (
      <div className="flex items-center justify-center min-h-screen bg-[hsl(220_20%_8%)]">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-[hsl(0_0%_40%)]">404</h1>
          <p className="text-[hsl(0_0%_40%)] mt-2">Page not found</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[hsl(220_20%_8%)] text-[hsl(0_0%_95%)]">
      <div className="container mx-auto py-8 px-4 max-w-6xl">
        {/* Header */}
        <div className="mb-8">
          <Link
            href="/dev"
            className="inline-flex items-center text-sm text-[hsl(0_0%_55%)] hover:text-[hsl(180_70%_45%)] mb-4 transition-colors"
          >
            <ArrowLeft className="w-4 h-4 mr-2" />
            Back to Developer Tools
          </Link>
          <div className="flex items-center gap-3 mb-2">
            <Terminal className="w-6 h-6 text-[hsl(180_70%_45%)]" />
            <h1 className="text-2xl font-bold">Design Concept: Terminal</h1>
          </div>
          <p className="text-[hsl(0_0%_55%)]">
            Sharp corners, cyan accents, dark-first, developer-focused aesthetic
          </p>
          <div className="flex gap-2 mt-3">
            <SharpBadge variant="accent">Sharp Corners</SharpBadge>
            <SharpBadge>Cyan Accent</SharpBadge>
            <SharpBadge>Dark Theme</SharpBadge>
          </div>
        </div>

        {/* Color Palette */}
        <section className="mb-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(180_70%_45%)]" />
            Color Palette
          </h2>
          <div className="grid grid-cols-6 gap-3">
            {[
              { name: "Accent", color: "hsl(180 70% 45%)", label: "Cyan" },
              { name: "Success", color: "hsl(145 70% 45%)", label: "Green" },
              { name: "Warning", color: "hsl(40 90% 50%)", label: "Amber" },
              { name: "Error", color: "hsl(0 70% 55%)", label: "Red" },
              { name: "Background", color: "hsl(220 20% 8%)", label: "Dark" },
              { name: "Card", color: "hsl(220 15% 12%)", label: "Elevated" },
            ].map((c) => (
              <div key={c.name} className="text-center">
                <div
                  className="w-full h-16 rounded-none border border-[hsl(220_15%_20%)] mb-2"
                  style={{ backgroundColor: c.color }}
                />
                <div className="text-xs font-medium">{c.name}</div>
                <div className="text-xs text-[hsl(0_0%_50%)]">{c.label}</div>
              </div>
            ))}
          </div>
        </section>

        {/* Components Grid */}
        <div className="grid grid-cols-2 gap-6">
          {/* Buttons */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[hsl(180_70%_45%)]" />
              Buttons
            </h2>
            <SharpCard className="p-6 space-y-4">
              <div className="flex gap-3 flex-wrap">
                <SharpButton variant="accent"><Play className="w-4 h-4" /> Run Agent</SharpButton>
                <SharpButton variant="default"><Plus className="w-4 h-4" /> New Session</SharpButton>
                <SharpButton variant="outline"><Settings className="w-4 h-4" /> Configure</SharpButton>
                <SharpButton variant="ghost"><Square className="w-4 h-4" /> Stop</SharpButton>
              </div>
              <div className="flex gap-3">
                <SharpButton variant="accent" size="sm">Small</SharpButton>
                <SharpButton variant="default" size="icon"><Search className="w-4 h-4" /></SharpButton>
                <SharpButton variant="ghost" size="icon"><Bell className="w-4 h-4" /></SharpButton>
              </div>
            </SharpCard>
          </section>

          {/* Inputs */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[hsl(180_70%_45%)]" />
              Inputs
            </h2>
            <SharpCard className="p-6 space-y-4">
              <SharpInput placeholder="Search agents..." />
              <SharpInput placeholder="Enter command..." />
              <div className="flex gap-3">
                <SharpInput placeholder="API Key" className="flex-1" />
                <SharpButton variant="accent">Save</SharpButton>
              </div>
            </SharpCard>
          </section>

          {/* Badges */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[hsl(180_70%_45%)]" />
              Badges
            </h2>
            <SharpCard className="p-6">
              <div className="flex gap-3 flex-wrap">
                <SharpBadge>default</SharpBadge>
                <SharpBadge variant="accent">running</SharpBadge>
                <SharpBadge variant="success">completed</SharpBadge>
                <SharpBadge variant="warning">pending</SharpBadge>
              </div>
            </SharpCard>
          </section>

          {/* Stats */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[hsl(180_70%_45%)]" />
              Stats Cards
            </h2>
            <div className="grid grid-cols-2 gap-3">
              <StatCard label="Active Agents" value="12" icon={Cpu} />
              <StatCard label="Sessions" value="847" icon={Activity} />
            </div>
          </section>
        </div>

        {/* Chat Preview */}
        <section className="mt-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(180_70%_45%)]" />
            Chat Interface
          </h2>
          <SharpCard>
            <div className="border-b border-[hsl(220_15%_20%)] px-4 py-3 flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="w-2 h-2 rounded-none bg-[hsl(180_70%_45%)] animate-pulse" />
                <span className="font-mono text-sm">ses_a7f3b2c</span>
                <SharpBadge variant="accent">running</SharpBadge>
              </div>
              <span className="text-xs text-[hsl(0_0%_50%)]">claude-3-5-sonnet</span>
            </div>
            <div className="p-4 space-y-4">
              <TerminalUserMessage content="Can you check the test results and fix any failures?" />
              <TerminalAgentMessage content="I'll run the test suite and analyze any failures." />
              <TerminalToolCall name="bash" status="done" result="running 24 tests... 23 passed, 1 failed" />
              <TerminalToolCall name="read_file" status="done" result="src/api/handler.rs:42" />
              <TerminalToolCall name="edit_file" status="running" />
              <TerminalAgentMessage content="Found the issue - there's a type mismatch on line 42. Fixing now..." />
            </div>
            <div className="border-t border-[hsl(220_15%_20%)] p-3">
              <div className="flex gap-2">
                <SharpInput placeholder="Type a message..." className="flex-1" />
                <SharpButton variant="accent">Send</SharpButton>
              </div>
            </div>
          </SharpCard>
        </section>

        {/* Full Layout Preview */}
        <section className="mt-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(180_70%_45%)]" />
            Full Layout Preview
          </h2>
          <SharpCard className="overflow-hidden">
            <div className="flex h-[500px]">
              {/* Sidebar */}
              <div className="w-56 border-r border-[hsl(220_15%_20%)] bg-[hsl(220_18%_10%)]">
                <div className="p-4 border-b border-[hsl(220_15%_20%)]">
                  <div className="flex items-center gap-2">
                    <EverrunsLogo size={28} accentColor="hsl(180 70% 45%)" />
                    <span className="font-semibold text-[hsl(180_70%_55%)]">everruns</span>
                  </div>
                </div>
                <div className="py-2">
                  <NavItem icon={Layers} label="Dashboard" />
                  <NavItem icon={Cpu} label="Agents" active />
                  <NavItem icon={Database} label="Sessions" />
                  <NavItem icon={Code} label="Capabilities" />
                  <NavItem icon={GitBranch} label="Workflows" />
                  <NavItem icon={Settings} label="Settings" />
                </div>
              </div>

              {/* Main content */}
              <div className="flex-1 flex flex-col">
                {/* Top bar */}
                <div className="h-12 border-b border-[hsl(220_15%_20%)] flex items-center justify-between px-4">
                  <div className="flex items-center gap-3">
                    <SharpInput placeholder="Search..." className="w-64" />
                  </div>
                  <div className="flex items-center gap-2">
                    <SharpButton variant="ghost" size="icon"><Bell className="w-4 h-4" /></SharpButton>
                    <SharpButton variant="ghost" size="icon"><User className="w-4 h-4" /></SharpButton>
                  </div>
                </div>

                {/* Content */}
                <div className="flex-1 p-6 overflow-auto">
                  <div className="flex items-center justify-between mb-6">
                    <h3 className="text-xl font-semibold">Agents</h3>
                    <SharpButton variant="accent"><Plus className="w-4 h-4" /> New Agent</SharpButton>
                  </div>

                  {/* Stats row */}
                  <div className="grid grid-cols-4 gap-4 mb-6">
                    <StatCard label="Total Agents" value="12" icon={Cpu} />
                    <StatCard label="Active" value="3" icon={Activity} />
                    <StatCard label="Sessions Today" value="47" icon={Database} />
                    <StatCard label="Tokens Used" value="1.2M" icon={Activity} />
                  </div>

                  {/* Session list */}
                  <SharpCard>
                    <div className="px-4 py-3 border-b border-[hsl(220_15%_20%)]">
                      <span className="font-medium text-sm">Recent Sessions</span>
                    </div>
                    <SessionRow id="ses_a7f3b2c" status="running" model="claude-3-5-sonnet" />
                    <SessionRow id="ses_b2e4d1a" status="completed" model="gpt-4o" />
                    <SessionRow id="ses_c9f8e3b" status="completed" model="claude-3-5-sonnet" />
                  </SharpCard>
                </div>
              </div>
            </div>
          </SharpCard>
        </section>

        {/* Design Notes */}
        <section className="mt-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(180_70%_45%)]" />
            Design Notes
          </h2>
          <SharpCard className="p-6">
            <div className="grid grid-cols-2 gap-6 text-sm">
              <div>
                <h3 className="font-medium text-[hsl(180_70%_55%)] mb-2">Philosophy</h3>
                <ul className="space-y-1 text-[hsl(0_0%_70%)]">
                  <li>- Zero border-radius for technical precision</li>
                  <li>- Cyan accent evokes terminal/CLI familiarity</li>
                  <li>- Dark-first: reduces eye strain for long sessions</li>
                  <li>- Monospace for IDs, commands, code</li>
                </ul>
              </div>
              <div>
                <h3 className="font-medium text-[hsl(180_70%_55%)] mb-2">Key Changes from Current</h3>
                <ul className="space-y-1 text-[hsl(0_0%_70%)]">
                  <li>- border-radius: 0 (was 0.5rem)</li>
                  <li>- Primary accent: Cyan (was grayscale)</li>
                  <li>- Background: hsl(220 20% 8%) (darker)</li>
                  <li>- More monospace typography</li>
                </ul>
              </div>
            </div>
          </SharpCard>
        </section>
      </div>
    </div>
  );
}
