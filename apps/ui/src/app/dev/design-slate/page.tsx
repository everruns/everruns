"use client";

import Link from "next/link";
import Image from "next/image";
import { ArrowLeft, Terminal, Check, Loader2, ChevronRight, Play, Square, Settings, Plus, Search, Bell, User, Database, Cpu, Activity, Code, Layers, GitBranch } from "lucide-react";
import { cn } from "@/lib/utils";

const isDev = process.env.NODE_ENV === "development";

/**
 * Design Concept: "Slate"
 *
 * Philosophy:
 * - Light mode with dotted background (navy dots per brand)
 * - Sharp corners (0px)
 * - Grayscale dominant UI
 * - Gold accent (#D4A43A) for interactive elements
 * - Original everruns logo (navy to gold)
 * - Clean, minimal, professional
 */

// Brand colors
const brand = {
  navy: "#0A1636",
  gold: "#D4A43A",
  goldHover: "#C49530",
  goldMuted: "hsl(43 60% 53% / 0.12)",
  goldBorder: "hsl(43 60% 53% / 0.3)",
};

// Dotted background style (navy dots per brand spec)
const dottedBg = {
  backgroundImage: `radial-gradient(circle at center, hsl(220 62% 13% / 0.08) 1px, transparent 1px)`,
  backgroundSize: '24px 24px',
};

// Button with subtle radius
function SlateButton({
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
    default: "bg-[#0A1636] text-white hover:bg-[#0D1D45]",
    accent: "text-[#0A1636] font-medium",
    ghost: "text-[hsl(0_0%_40%)] hover:text-[hsl(0_0%_10%)] hover:bg-[hsl(0_0%_92%)]",
    outline: "border border-[hsl(0_0%_80%)] text-[hsl(0_0%_30%)] hover:border-[#D4A43A] hover:text-[#C49530]",
  };

  const sizes = {
    default: "px-4 py-2 text-sm",
    sm: "px-3 py-1.5 text-xs",
    icon: "p-2",
  };

  const accentStyles = variant === "accent"
    ? { backgroundColor: brand.gold }
    : {};

  return (
    <button
      className={cn(
        "inline-flex items-center justify-center gap-2 transition-colors",
        variants[variant],
        sizes[size],
        className
      )}
      style={{ borderRadius: '0px', ...accentStyles }}
    >
      {children}
    </button>
  );
}

// Card with subtle radius
function SlateCard({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "border border-[hsl(0_0%_88%)] bg-white",
        className
      )}
      style={{ borderRadius: '0px' }}
    >
      {children}
    </div>
  );
}

// Input with subtle radius
function SlateInput({ placeholder, className = "" }: { placeholder?: string; className?: string }) {
  return (
    <input
      type="text"
      placeholder={placeholder}
      className={cn(
        "w-full border border-[hsl(0_0%_85%)] bg-white px-3 py-2 text-sm",
        "text-[hsl(0_0%_10%)] placeholder:text-[hsl(0_0%_55%)]",
        "focus:outline-none focus:border-[#D4A43A] focus:ring-1 focus:ring-[#D4A43A40]",
        className
      )}
      style={{ borderRadius: '0px' }}
    />
  );
}

// Badge with subtle radius
function SlateBadge({
  children,
  variant = "default"
}: {
  children: React.ReactNode;
  variant?: "default" | "success" | "warning" | "accent";
}) {
  const variants = {
    default: "bg-[hsl(0_0%_94%)] text-[hsl(0_0%_40%)] border-[hsl(0_0%_88%)]",
    success: "bg-[hsl(145_50%_94%)] text-[hsl(145_60%_30%)] border-[hsl(145_50%_80%)]",
    warning: "bg-[hsl(40_80%_94%)] text-[hsl(40_80%_30%)] border-[hsl(40_80%_80%)]",
    accent: "bg-[#D4A43A20] text-[#9A7420] border-[#D4A43A50]",
  };

  return (
    <span
      className={cn(
        "inline-flex items-center px-2 py-0.5 text-xs font-medium border",
        variants[variant]
      )}
      style={{ borderRadius: '0px' }}
    >
      {children}
    </span>
  );
}

// Chat components
function SlateUserMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] border-r-2 border-[#D4A43A] bg-[#D4A43A10] px-3 py-2">
        <p className="text-sm text-[hsl(0_0%_15%)]">{content}</p>
      </div>
    </div>
  );
}

function SlateAgentMessage({ content }: { content: string }) {
  return (
    <div className="flex gap-2">
      <div
        className="w-5 h-5 bg-[hsl(0_0%_94%)] border border-[hsl(0_0%_85%)] flex items-center justify-center flex-shrink-0 mt-0.5"
        style={{ borderRadius: '0px' }}
      >
        <Terminal className="w-3 h-3 text-[hsl(0_0%_40%)]" />
      </div>
      <p className="text-sm text-[hsl(0_0%_25%)]">{content}</p>
    </div>
  );
}

function SlateToolCall({ name, status, result }: { name: string; status: "done" | "running"; result?: string }) {
  return (
    <div className="ml-7 text-xs">
      <div className="flex items-center gap-2 text-[hsl(0_0%_50%)]">
        {status === "running" ? (
          <Loader2 className="w-3 h-3 animate-spin text-[#D4A43A]" />
        ) : (
          <Check className="w-3 h-3 text-[hsl(145_60%_40%)]" />
        )}
        <span className="font-mono text-[hsl(0_0%_30%)]">{name}</span>
      </div>
      {result && (
        <div
          className="mt-1 ml-5 px-2 py-1 text-[hsl(0_0%_45%)] bg-[hsl(0_0%_97%)] border-l-2 border-[hsl(0_0%_85%)]"
        >
          {result}
        </div>
      )}
    </div>
  );
}

// Nav item
function NavItem({ icon: Icon, label, active = false }: { icon: React.ElementType; label: string; active?: boolean }) {
  return (
    <div
      className={cn(
        "flex items-center gap-3 px-3 py-2 text-sm cursor-pointer transition-colors",
        active
          ? "bg-[#D4A43A18] text-[#9A7420] border-l-2 border-[#D4A43A]"
          : "text-[hsl(0_0%_45%)] hover:text-[hsl(0_0%_15%)] hover:bg-[hsl(0_0%_94%)]"
      )}
    >
      <Icon className="w-4 h-4" />
      <span>{label}</span>
    </div>
  );
}

// Stats card
function StatCard({ label, value, icon: Icon, trend }: { label: string; value: string; icon: React.ElementType; trend?: string }) {
  return (
    <SlateCard className="p-4">
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs text-[hsl(0_0%_50%)] uppercase tracking-wide">{label}</span>
        <Icon className="w-4 h-4 text-[hsl(0_0%_60%)]" />
      </div>
      <div className="flex items-baseline gap-2">
        <div className="text-2xl font-semibold text-[hsl(0_0%_10%)]">{value}</div>
        {trend && <span className="text-xs text-[hsl(145_60%_40%)]">{trend}</span>}
      </div>
    </SlateCard>
  );
}

// Session row
function SessionRow({ id, status, model, time }: { id: string; status: "running" | "completed"; model: string; time: string }) {
  return (
    <div className="flex items-center justify-between px-4 py-3 border-b border-[hsl(0_0%_92%)] hover:bg-[hsl(0_0%_97%)] cursor-pointer transition-colors">
      <div className="flex items-center gap-3">
        {status === "running" ? (
          <div className="w-2 h-2 rounded-full bg-[#D4A43A] animate-pulse" />
        ) : (
          <div className="w-2 h-2 rounded-full bg-[hsl(145_60%_45%)]" />
        )}
        <span className="font-mono text-sm text-[hsl(0_0%_25%)]">{id}</span>
        <span className="text-xs text-[hsl(0_0%_55%)]">{model}</span>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-xs text-[hsl(0_0%_60%)]">{time}</span>
        <ChevronRight className="w-4 h-4 text-[hsl(0_0%_70%)]" />
      </div>
    </div>
  );
}

export default function DesignSlatePage() {
  if (!isDev) {
    return (
      <div className="flex items-center justify-center min-h-screen bg-white">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-[hsl(0_0%_70%)]">404</h1>
          <p className="text-[hsl(0_0%_70%)] mt-2">Page not found</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[hsl(0_0%_98%)] text-[hsl(0_0%_10%)]" style={dottedBg}>
      <div className="container mx-auto py-8 px-4 max-w-6xl">
        {/* Header */}
        <div className="mb-8">
          <Link
            href="/dev"
            className="inline-flex items-center text-sm text-[hsl(0_0%_50%)] hover:text-[#D4A43A] mb-4 transition-colors"
          >
            <ArrowLeft className="w-4 h-4 mr-2" />
            Back to Developer Tools
          </Link>
          <div className="flex items-center gap-3 mb-2">
            <Image src="/logo.svg" alt="Everruns" width={32} height={32} />
            <h1 className="text-2xl font-bold">Design Concept: Slate</h1>
          </div>
          <p className="text-[hsl(0_0%_45%)]">
            Light mode with navy dots, sharp corners, gold accent from brand
          </p>
          <div className="flex gap-2 mt-3">
            <SlateBadge variant="accent">Sharp Corners</SlateBadge>
            <SlateBadge>Light + Dots</SlateBadge>
            <SlateBadge>Gold Accent</SlateBadge>
          </div>
        </div>

        {/* Color Palette */}
        <section className="mb-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[#0A1636]" style={{ borderRadius: '1px' }} />
            Brand Color Palette
          </h2>
          <div className="grid grid-cols-7 gap-3">
            {[
              { name: "Navy", color: "#0A1636", label: "Primary" },
              { name: "Gold", color: "#D4A43A", label: "Accent" },
              { name: "Background", color: "hsl(0 0% 98%)", label: "Off White", border: true },
              { name: "Card", color: "#FFFFFF", label: "White", border: true },
              { name: "Charcoal", color: "#1A1A1A", label: "Text" },
              { name: "Slate", color: "#404040", label: "Secondary" },
              { name: "Silver", color: "#A0A0A0", label: "Muted" },
            ].map((c) => (
              <div key={c.name} className="text-center">
                <div
                  className={cn("w-full h-16 mb-2", c.border && "border border-[hsl(0_0%_85%)]")}
                  style={{ backgroundColor: c.color, borderRadius: '0px' }}
                />
                <div className="text-xs font-medium">{c.name}</div>
                <div className="text-xs text-[hsl(0_0%_55%)]">{c.label}</div>
              </div>
            ))}
          </div>
        </section>

        {/* Accent Usage */}
        <section className="mb-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[#D4A43A]" style={{ borderRadius: '1px' }} />
            Gold Accent Usage
          </h2>
          <SlateCard className="p-6">
            <p className="text-sm text-[hsl(0_0%_45%)] mb-4">
              Gold (#D4A43A) is used for interactive and active states. Navy (#0A1636) for primary actions.
            </p>
            <div className="grid grid-cols-4 gap-4 text-sm">
              <div>
                <div className="text-[hsl(0_0%_55%)] mb-2">Gold button</div>
                <SlateButton variant="accent">Run Agent</SlateButton>
              </div>
              <div>
                <div className="text-[hsl(0_0%_55%)] mb-2">Active nav item</div>
                <div className="bg-[#D4A43A18] text-[#9A7420] border-l-2 border-[#D4A43A] px-3 py-2 text-sm">
                  Agents
                </div>
              </div>
              <div>
                <div className="text-[hsl(0_0%_55%)] mb-2">Running status</div>
                <div className="flex items-center gap-2">
                  <div className="w-2 h-2 rounded-full bg-[#D4A43A] animate-pulse" />
                  <span className="text-[hsl(0_0%_30%)]">running</span>
                </div>
              </div>
              <div>
                <div className="text-[hsl(0_0%_55%)] mb-2">Focus ring</div>
                <input
                  className="w-full border-2 border-[#D4A43A] bg-white px-3 py-2 text-sm text-[hsl(0_0%_10%)]"
                  style={{ borderRadius: '0px' }}
                  defaultValue="Focused"
                  readOnly
                />
              </div>
            </div>
          </SlateCard>
        </section>

        {/* Components Grid */}
        <div className="grid grid-cols-2 gap-6">
          {/* Buttons */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[#0A1636]" style={{ borderRadius: '1px' }} />
              Buttons
            </h2>
            <SlateCard className="p-6 space-y-4">
              <div className="flex gap-3 flex-wrap">
                <SlateButton variant="accent"><Play className="w-4 h-4" /> Run Agent</SlateButton>
                <SlateButton variant="default"><Plus className="w-4 h-4" /> New Session</SlateButton>
                <SlateButton variant="outline"><Settings className="w-4 h-4" /> Configure</SlateButton>
                <SlateButton variant="ghost"><Square className="w-4 h-4" /> Stop</SlateButton>
              </div>
              <div className="flex gap-3">
                <SlateButton variant="accent" size="sm">Small</SlateButton>
                <SlateButton variant="default" size="icon"><Search className="w-4 h-4" /></SlateButton>
                <SlateButton variant="ghost" size="icon"><Bell className="w-4 h-4" /></SlateButton>
              </div>
            </SlateCard>
          </section>

          {/* Inputs */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[#0A1636]" style={{ borderRadius: '1px' }} />
              Inputs
            </h2>
            <SlateCard className="p-6 space-y-4">
              <SlateInput placeholder="Search agents..." />
              <SlateInput placeholder="Enter command..." />
              <div className="flex gap-3">
                <SlateInput placeholder="API Key" className="flex-1" />
                <SlateButton variant="accent">Save</SlateButton>
              </div>
            </SlateCard>
          </section>

          {/* Badges */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[#0A1636]" style={{ borderRadius: '1px' }} />
              Badges & Status
            </h2>
            <SlateCard className="p-6">
              <div className="flex gap-3 flex-wrap mb-4">
                <SlateBadge>default</SlateBadge>
                <SlateBadge variant="accent">running</SlateBadge>
                <SlateBadge variant="success">completed</SlateBadge>
                <SlateBadge variant="warning">pending</SlateBadge>
              </div>
              <div className="flex items-center gap-4 text-xs text-[hsl(0_0%_50%)]">
                <div className="flex items-center gap-2">
                  <div className="w-2 h-2 rounded-full bg-[#D4A43A] animate-pulse" />
                  <span>Running</span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="w-2 h-2 rounded-full bg-[hsl(145_60%_45%)]" />
                  <span>Completed</span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="w-2 h-2 rounded-full bg-[hsl(0_0%_75%)]" />
                  <span>Idle</span>
                </div>
              </div>
            </SlateCard>
          </section>

          {/* Stats */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[#0A1636]" style={{ borderRadius: '1px' }} />
              Stats Cards
            </h2>
            <div className="grid grid-cols-2 gap-3">
              <StatCard label="Active Agents" value="12" icon={Cpu} />
              <StatCard label="Sessions" value="847" icon={Activity} trend="+23" />
            </div>
          </section>
        </div>

        {/* Chat Preview */}
        <section className="mt-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[#0A1636]" style={{ borderRadius: '1px' }} />
            Chat Interface
          </h2>
          <SlateCard>
            <div className="border-b border-[hsl(0_0%_92%)] px-4 py-3 flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="w-2 h-2 rounded-full bg-[#D4A43A] animate-pulse" />
                <span className="font-mono text-sm">ses_a7f3b2c</span>
                <SlateBadge variant="accent">running</SlateBadge>
              </div>
              <span className="text-xs text-[hsl(0_0%_55%)]">claude-3-5-sonnet</span>
            </div>
            <div className="p-4 space-y-4 bg-[hsl(0_0%_99%)]">
              <SlateUserMessage content="Can you check the test results and fix any failures?" />
              <SlateAgentMessage content="I'll run the test suite and analyze any failures." />
              <SlateToolCall name="bash" status="done" result="running 24 tests... 23 passed, 1 failed" />
              <SlateToolCall name="read_file" status="done" result="src/api/handler.rs:42" />
              <SlateToolCall name="edit_file" status="running" />
              <SlateAgentMessage content="Found the issue - there's a type mismatch on line 42. Fixing now..." />
            </div>
            <div className="border-t border-[hsl(0_0%_92%)] p-3">
              <div className="flex gap-2">
                <SlateInput placeholder="Type a message..." className="flex-1" />
                <SlateButton variant="accent">Send</SlateButton>
              </div>
            </div>
          </SlateCard>
        </section>

        {/* Full Layout Preview */}
        <section className="mt-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[#0A1636]" style={{ borderRadius: '1px' }} />
            Full Layout Preview
          </h2>
          <SlateCard className="overflow-hidden">
            <div className="flex h-[500px]">
              {/* Sidebar */}
              <div className="w-56 border-r border-[hsl(0_0%_92%)] bg-white">
                <div className="p-4 border-b border-[hsl(0_0%_92%)]">
                  <div className="flex items-center gap-2">
                    <Image src="/logo.svg" alt="Everruns" width={28} height={28} />
                    <span className="font-semibold text-[#0A1636]">everruns</span>
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
                <div className="h-12 border-b border-[hsl(0_0%_92%)] flex items-center justify-between px-4 bg-white">
                  <div className="flex items-center gap-3">
                    <SlateInput placeholder="Search..." className="w-64" />
                  </div>
                  <div className="flex items-center gap-2">
                    <SlateButton variant="ghost" size="icon"><Bell className="w-4 h-4" /></SlateButton>
                    <SlateButton variant="ghost" size="icon"><User className="w-4 h-4" /></SlateButton>
                  </div>
                </div>

                {/* Content */}
                <div className="flex-1 p-6 overflow-auto bg-[hsl(0_0%_98%)]" style={dottedBg}>
                  <div className="flex items-center justify-between mb-6">
                    <h3 className="text-xl font-semibold">Agents</h3>
                    <SlateButton variant="accent"><Plus className="w-4 h-4" /> New Agent</SlateButton>
                  </div>

                  {/* Stats row */}
                  <div className="grid grid-cols-4 gap-4 mb-6">
                    <StatCard label="Total Agents" value="12" icon={Cpu} />
                    <StatCard label="Active" value="3" icon={Activity} trend="+2" />
                    <StatCard label="Sessions" value="847" icon={Database} trend="+47" />
                    <StatCard label="Tokens" value="1.2M" icon={Code} />
                  </div>

                  {/* Session list */}
                  <SlateCard>
                    <div className="px-4 py-3 border-b border-[hsl(0_0%_92%)]">
                      <span className="font-medium text-sm">Recent Sessions</span>
                    </div>
                    <SessionRow id="ses_a7f3b2c" status="running" model="claude-sonnet" time="2m ago" />
                    <SessionRow id="ses_b2e4d1a" status="completed" model="gpt-4o" time="15m ago" />
                    <SessionRow id="ses_c9f8e3b" status="completed" model="claude-sonnet" time="1h ago" />
                  </SlateCard>
                </div>
              </div>
            </div>
          </SlateCard>
        </section>

        {/* Design Notes */}
        <section className="mt-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[#0A1636]" style={{ borderRadius: '1px' }} />
            Design Notes
          </h2>
          <SlateCard className="p-6">
            <div className="grid grid-cols-2 gap-6 text-sm">
              <div>
                <h3 className="font-medium text-[hsl(0_0%_20%)] mb-2">Brand Alignment</h3>
                <ul className="space-y-1 text-[hsl(0_0%_45%)]">
                  <li>- Uses original everruns logo</li>
                  <li>- Navy (#0A1636) for primary actions</li>
                  <li>- Gold (#D4A43A) for accents & highlights</li>
                  <li>- Navy dot pattern on backgrounds</li>
                </ul>
              </div>
              <div>
                <h3 className="font-medium text-[hsl(0_0%_20%)] mb-2">UI Refinements</h3>
                <ul className="space-y-1 text-[hsl(0_0%_45%)]">
                  <li>- 0px border radius (sharp corners)</li>
                  <li>- Grayscale dominant, accents sparingly</li>
                  <li>- Gold for running/active states</li>
                  <li>- Clean hierarchy, minimal chrome</li>
                </ul>
              </div>
            </div>
          </SlateCard>
        </section>
      </div>
    </div>
  );
}
