"use client";

import Link from "next/link";
import { ArrowLeft, Terminal, Check, Loader2, ChevronRight, Play, Square, Settings, Plus, Search, Bell, User, Database, Cpu, Activity, Code, Layers, GitBranch } from "lucide-react";
import { cn } from "@/lib/utils";

const isDev = process.env.NODE_ENV === "development";

// Everruns logo - 3 interlocking circles
function EverrunsLogo({ size = 24, accentColor = "hsl(210 60% 50%)" }: { size?: number; accentColor?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 512 512">
      <defs>
        <linearGradient id="gTopSlate" gradientUnits="userSpaceOnUse" x1="256" y1="94" x2="256" y2="284">
          <stop offset="0.00" stopColor={accentColor} stopOpacity="0.3" />
          <stop offset="0.70" stopColor={accentColor} stopOpacity="0.6" />
          <stop offset="1.00" stopColor={accentColor} />
        </linearGradient>
        <linearGradient id="gLeftSlate" gradientUnits="userSpaceOnUse" x1="70" y1="374" x2="256" y2="284">
          <stop offset="0.00" stopColor={accentColor} stopOpacity="0.2" />
          <stop offset="0.70" stopColor={accentColor} stopOpacity="0.5" />
          <stop offset="1.00" stopColor={accentColor} />
        </linearGradient>
        <linearGradient id="gRightSlate" gradientUnits="userSpaceOnUse" x1="442" y1="374" x2="256" y2="284">
          <stop offset="0.00" stopColor={accentColor} stopOpacity="0.15" />
          <stop offset="0.70" stopColor={accentColor} stopOpacity="0.4" />
          <stop offset="1.00" stopColor={accentColor} />
        </linearGradient>
      </defs>
      <g fill="none" strokeWidth="18" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="256" cy="214" r="120" stroke="url(#gTopSlate)" />
        <circle cx="186" cy="309" r="120" stroke="url(#gLeftSlate)" />
        <circle cx="326" cy="309" r="120" stroke="url(#gRightSlate)" />
      </g>
    </svg>
  );
}

/**
 * Design Concept: "Slate"
 *
 * Philosophy:
 * - Subtle corners (4px) - between current (8px) and Terminal (0px)
 * - Muted blue accent - professional, not flashy
 * - Balanced contrast - comfortable for long sessions
 * - Clean lines with just enough softness
 * - Developer-focused but approachable
 */

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
    default: "bg-[hsl(215_15%_18%)] text-[hsl(210_10%_90%)] hover:bg-[hsl(215_15%_22%)] border border-[hsl(215_15%_25%)]",
    accent: "bg-[hsl(210_60%_50%)] text-white hover:bg-[hsl(210_60%_55%)] font-medium",
    ghost: "text-[hsl(210_10%_60%)] hover:text-[hsl(210_10%_90%)] hover:bg-[hsl(215_15%_15%)]",
    outline: "border border-[hsl(210_60%_50%/_0.4)] text-[hsl(210_60%_60%)] hover:bg-[hsl(210_60%_50%/_0.1)]",
  };

  const sizes = {
    default: "px-4 py-2 text-sm",
    sm: "px-3 py-1.5 text-xs",
    icon: "p-2",
  };

  return (
    <button className={cn(
      "inline-flex items-center justify-center gap-2 rounded transition-colors",
      variants[variant],
      sizes[size],
      className
    )}
    style={{ borderRadius: '4px' }}
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
        "border border-[hsl(215_15%_20%)] bg-[hsl(215_15%_11%)]",
        className
      )}
      style={{ borderRadius: '4px' }}
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
        "w-full border border-[hsl(215_15%_20%)] bg-[hsl(215_15%_8%)] px-3 py-2 text-sm",
        "text-[hsl(210_10%_90%)] placeholder:text-[hsl(210_10%_40%)]",
        "focus:outline-none focus:border-[hsl(210_60%_50%)] focus:ring-1 focus:ring-[hsl(210_60%_50%/_0.3)]",
        className
      )}
      style={{ borderRadius: '4px' }}
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
    default: "bg-[hsl(215_15%_18%)] text-[hsl(210_10%_60%)] border-[hsl(215_15%_25%)]",
    success: "bg-[hsl(145_50%_40%/_0.15)] text-[hsl(145_50%_55%)] border-[hsl(145_50%_40%/_0.3)]",
    warning: "bg-[hsl(40_80%_50%/_0.15)] text-[hsl(40_80%_60%)] border-[hsl(40_80%_50%/_0.3)]",
    accent: "bg-[hsl(210_60%_50%/_0.15)] text-[hsl(210_60%_65%)] border-[hsl(210_60%_50%/_0.3)]",
  };

  return (
    <span
      className={cn(
        "inline-flex items-center px-2 py-0.5 text-xs font-medium border",
        variants[variant]
      )}
      style={{ borderRadius: '4px' }}
    >
      {children}
    </span>
  );
}

// Chat components
function SlateUserMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div
        className="max-w-[85%] border border-[hsl(210_60%_50%/_0.3)] bg-[hsl(210_60%_50%/_0.08)] px-3 py-2"
        style={{ borderRadius: '6px' }}
      >
        <p className="text-sm text-[hsl(210_10%_95%)]">{content}</p>
      </div>
    </div>
  );
}

function SlateAgentMessage({ content }: { content: string }) {
  return (
    <div className="flex gap-2">
      <div
        className="w-5 h-5 bg-[hsl(210_60%_50%/_0.15)] border border-[hsl(210_60%_50%/_0.3)] flex items-center justify-center flex-shrink-0 mt-0.5"
        style={{ borderRadius: '4px' }}
      >
        <Terminal className="w-3 h-3 text-[hsl(210_60%_55%)]" />
      </div>
      <p className="text-sm text-[hsl(210_10%_80%)]">{content}</p>
    </div>
  );
}

function SlateToolCall({ name, status, result }: { name: string; status: "done" | "running"; result?: string }) {
  return (
    <div className="ml-7 text-xs">
      <div className="flex items-center gap-2 text-[hsl(210_10%_55%)]">
        {status === "running" ? (
          <Loader2 className="w-3 h-3 animate-spin text-[hsl(210_60%_55%)]" />
        ) : (
          <Check className="w-3 h-3 text-[hsl(145_50%_50%)]" />
        )}
        <span className="font-mono text-[hsl(210_60%_60%)]">{name}</span>
      </div>
      {result && (
        <div
          className="mt-1 ml-5 px-2 py-1 text-[hsl(210_10%_50%)] bg-[hsl(215_15%_10%)] border-l-2 border-[hsl(215_15%_25%)]"
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
        "flex items-center gap-3 px-3 py-2 text-sm cursor-pointer transition-colors mx-2",
        active
          ? "bg-[hsl(210_60%_50%/_0.12)] text-[hsl(210_60%_65%)]"
          : "text-[hsl(210_10%_55%)] hover:text-[hsl(210_10%_85%)] hover:bg-[hsl(215_15%_14%)]"
      )}
      style={{ borderRadius: '4px' }}
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
        <span className="text-xs text-[hsl(210_10%_50%)] uppercase tracking-wide">{label}</span>
        <Icon className="w-4 h-4 text-[hsl(210_60%_55%)]" />
      </div>
      <div className="flex items-baseline gap-2">
        <div className="text-2xl font-semibold text-[hsl(210_10%_90%)]">{value}</div>
        {trend && <span className="text-xs text-[hsl(145_50%_55%)]">{trend}</span>}
      </div>
    </SlateCard>
  );
}

// Session row
function SessionRow({ id, status, model, time }: { id: string; status: "running" | "completed"; model: string; time: string }) {
  return (
    <div className="flex items-center justify-between px-4 py-3 border-b border-[hsl(215_15%_15%)] hover:bg-[hsl(215_15%_13%)] cursor-pointer transition-colors">
      <div className="flex items-center gap-3">
        {status === "running" ? (
          <div className="w-2 h-2 rounded-full bg-[hsl(210_60%_55%)] animate-pulse" />
        ) : (
          <div className="w-2 h-2 rounded-full bg-[hsl(145_50%_50%)]" />
        )}
        <span className="font-mono text-sm text-[hsl(210_10%_80%)]">{id}</span>
        <span className="text-xs text-[hsl(210_10%_50%)]">{model}</span>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-xs text-[hsl(210_10%_45%)]">{time}</span>
        <ChevronRight className="w-4 h-4 text-[hsl(210_10%_35%)]" />
      </div>
    </div>
  );
}

export default function DesignSlatePage() {
  if (!isDev) {
    return (
      <div className="flex items-center justify-center min-h-screen bg-[hsl(215_15%_9%)]">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-[hsl(210_10%_35%)]">404</h1>
          <p className="text-[hsl(210_10%_35%)] mt-2">Page not found</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[hsl(215_15%_9%)] text-[hsl(210_10%_90%)]">
      <div className="container mx-auto py-8 px-4 max-w-6xl">
        {/* Header */}
        <div className="mb-8">
          <Link
            href="/dev"
            className="inline-flex items-center text-sm text-[hsl(210_10%_55%)] hover:text-[hsl(210_60%_55%)] mb-4 transition-colors"
          >
            <ArrowLeft className="w-4 h-4 mr-2" />
            Back to Developer Tools
          </Link>
          <div className="flex items-center gap-3 mb-2">
            <EverrunsLogo size={28} accentColor="hsl(210 60% 50%)" />
            <h1 className="text-2xl font-bold">Design Concept: Slate</h1>
          </div>
          <p className="text-[hsl(210_10%_55%)]">
            Balanced refinement - subtle 4px corners, muted blue accent, professional developer aesthetic
          </p>
          <div className="flex gap-2 mt-3">
            <SlateBadge variant="accent">Subtle Corners (4px)</SlateBadge>
            <SlateBadge>Blue Accent</SlateBadge>
            <SlateBadge>Balanced Contrast</SlateBadge>
          </div>
        </div>

        {/* Color Palette */}
        <section className="mb-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(210_60%_50%)]" style={{ borderRadius: '1px' }} />
            Color Palette
          </h2>
          <div className="grid grid-cols-6 gap-3">
            {[
              { name: "Accent", color: "hsl(210 60% 50%)", label: "Blue" },
              { name: "Success", color: "hsl(145 50% 45%)", label: "Green" },
              { name: "Warning", color: "hsl(40 80% 50%)", label: "Amber" },
              { name: "Error", color: "hsl(0 65% 55%)", label: "Red" },
              { name: "Background", color: "hsl(215 15% 9%)", label: "Slate" },
              { name: "Card", color: "hsl(215 15% 11%)", label: "Elevated" },
            ].map((c) => (
              <div key={c.name} className="text-center">
                <div
                  className="w-full h-16 border border-[hsl(215_15%_20%)] mb-2"
                  style={{ backgroundColor: c.color, borderRadius: '4px' }}
                />
                <div className="text-xs font-medium">{c.name}</div>
                <div className="text-xs text-[hsl(210_10%_50%)]">{c.label}</div>
              </div>
            ))}
          </div>
        </section>

        {/* Comparison table */}
        <section className="mb-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(210_60%_50%)]" style={{ borderRadius: '1px' }} />
            Design Spectrum
          </h2>
          <SlateCard className="p-6">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-[hsl(210_10%_50%)]">
                  <th className="pb-3 font-medium">Property</th>
                  <th className="pb-3 font-medium">Current</th>
                  <th className="pb-3 font-medium text-[hsl(210_60%_60%)]">Slate (this)</th>
                  <th className="pb-3 font-medium">Terminal</th>
                </tr>
              </thead>
              <tbody className="text-[hsl(210_10%_70%)]">
                <tr className="border-t border-[hsl(215_15%_15%)]">
                  <td className="py-2">Border Radius</td>
                  <td className="py-2">8px (rounded)</td>
                  <td className="py-2 text-[hsl(210_60%_60%)]">4px (subtle)</td>
                  <td className="py-2">0px (sharp)</td>
                </tr>
                <tr className="border-t border-[hsl(215_15%_15%)]">
                  <td className="py-2">Primary Accent</td>
                  <td className="py-2">Grayscale</td>
                  <td className="py-2 text-[hsl(210_60%_60%)]">Blue hsl(210 60% 50%)</td>
                  <td className="py-2">Cyan hsl(180 70% 45%)</td>
                </tr>
                <tr className="border-t border-[hsl(215_15%_15%)]">
                  <td className="py-2">Background</td>
                  <td className="py-2">Light/Dark adaptive</td>
                  <td className="py-2 text-[hsl(210_60%_60%)]">Slate hsl(215 15% 9%)</td>
                  <td className="py-2">Dark hsl(220 20% 8%)</td>
                </tr>
                <tr className="border-t border-[hsl(215_15%_15%)]">
                  <td className="py-2">Feel</td>
                  <td className="py-2">Neutral, accessible</td>
                  <td className="py-2 text-[hsl(210_60%_60%)]">Professional, refined</td>
                  <td className="py-2">Technical, hacker</td>
                </tr>
                <tr className="border-t border-[hsl(215_15%_15%)]">
                  <td className="py-2">Typography</td>
                  <td className="py-2">System sans-serif</td>
                  <td className="py-2 text-[hsl(210_60%_60%)]">Sans + mono for code</td>
                  <td className="py-2">Heavy monospace</td>
                </tr>
              </tbody>
            </table>
          </SlateCard>
        </section>

        {/* Components Grid */}
        <div className="grid grid-cols-2 gap-6">
          {/* Buttons */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[hsl(210_60%_50%)]" style={{ borderRadius: '1px' }} />
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
              <span className="w-1 h-4 bg-[hsl(210_60%_50%)]" style={{ borderRadius: '1px' }} />
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
              <span className="w-1 h-4 bg-[hsl(210_60%_50%)]" style={{ borderRadius: '1px' }} />
              Badges & Status
            </h2>
            <SlateCard className="p-6">
              <div className="flex gap-3 flex-wrap mb-4">
                <SlateBadge>default</SlateBadge>
                <SlateBadge variant="accent">running</SlateBadge>
                <SlateBadge variant="success">completed</SlateBadge>
                <SlateBadge variant="warning">pending</SlateBadge>
              </div>
              <div className="flex items-center gap-4 text-xs text-[hsl(210_10%_55%)]">
                <div className="flex items-center gap-2">
                  <div className="w-2 h-2 rounded-full bg-[hsl(210_60%_55%)] animate-pulse" />
                  <span>Running</span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="w-2 h-2 rounded-full bg-[hsl(145_50%_50%)]" />
                  <span>Completed</span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="w-2 h-2 rounded-full bg-[hsl(210_10%_35%)]" />
                  <span>Idle</span>
                </div>
              </div>
            </SlateCard>
          </section>

          {/* Stats */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[hsl(210_60%_50%)]" style={{ borderRadius: '1px' }} />
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
            <span className="w-1 h-4 bg-[hsl(210_60%_50%)]" style={{ borderRadius: '1px' }} />
            Chat Interface
          </h2>
          <SlateCard>
            <div className="border-b border-[hsl(215_15%_18%)] px-4 py-3 flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="w-2 h-2 rounded-full bg-[hsl(210_60%_55%)] animate-pulse" />
                <span className="font-mono text-sm">ses_a7f3b2c</span>
                <SlateBadge variant="accent">running</SlateBadge>
              </div>
              <span className="text-xs text-[hsl(210_10%_50%)]">claude-3-5-sonnet</span>
            </div>
            <div className="p-4 space-y-4">
              <SlateUserMessage content="Can you check the test results and fix any failures?" />
              <SlateAgentMessage content="I'll run the test suite and analyze any failures." />
              <SlateToolCall name="bash" status="done" result="running 24 tests... 23 passed, 1 failed" />
              <SlateToolCall name="read_file" status="done" result="src/api/handler.rs:42" />
              <SlateToolCall name="edit_file" status="running" />
              <SlateAgentMessage content="Found the issue - there's a type mismatch on line 42. Fixing now..." />
            </div>
            <div className="border-t border-[hsl(215_15%_18%)] p-3">
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
            <span className="w-1 h-4 bg-[hsl(210_60%_50%)]" style={{ borderRadius: '1px' }} />
            Full Layout Preview
          </h2>
          <SlateCard className="overflow-hidden">
            <div className="flex h-[500px]">
              {/* Sidebar */}
              <div className="w-56 border-r border-[hsl(215_15%_18%)] bg-[hsl(215_15%_10%)]">
                <div className="p-4 border-b border-[hsl(215_15%_18%)]">
                  <div className="flex items-center gap-2">
                    <EverrunsLogo size={28} accentColor="hsl(210 60% 50%)" />
                    <span className="font-semibold text-[hsl(210_60%_60%)]">everruns</span>
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
                <div className="h-12 border-b border-[hsl(215_15%_18%)] flex items-center justify-between px-4 bg-[hsl(215_15%_10%)]">
                  <div className="flex items-center gap-3">
                    <SlateInput placeholder="Search..." className="w-64" />
                  </div>
                  <div className="flex items-center gap-2">
                    <SlateButton variant="ghost" size="icon"><Bell className="w-4 h-4" /></SlateButton>
                    <SlateButton variant="ghost" size="icon"><User className="w-4 h-4" /></SlateButton>
                  </div>
                </div>

                {/* Content */}
                <div className="flex-1 p-6 overflow-auto bg-[hsl(215_15%_8%)]">
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
                    <div className="px-4 py-3 border-b border-[hsl(215_15%_18%)]">
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
            <span className="w-1 h-4 bg-[hsl(210_60%_50%)]" style={{ borderRadius: '1px' }} />
            Design Notes
          </h2>
          <SlateCard className="p-6">
            <div className="grid grid-cols-2 gap-6 text-sm">
              <div>
                <h3 className="font-medium text-[hsl(210_60%_60%)] mb-2">Philosophy</h3>
                <ul className="space-y-1 text-[hsl(210_10%_65%)]">
                  <li>- 4px radius: technical but not harsh</li>
                  <li>- Muted blue: professional, not flashy</li>
                  <li>- Balanced contrast for comfort</li>
                  <li>- Developer-focused yet approachable</li>
                </ul>
              </div>
              <div>
                <h3 className="font-medium text-[hsl(210_60%_60%)] mb-2">Key Differentiators</h3>
                <ul className="space-y-1 text-[hsl(210_10%_65%)]">
                  <li>- Middle ground between soft and sharp</li>
                  <li>- Blue accent feels familiar (IDE-like)</li>
                  <li>- Slate background is easier on eyes</li>
                  <li>- Clean without being sterile</li>
                </ul>
              </div>
            </div>
          </SlateCard>
        </section>
      </div>
    </div>
  );
}
