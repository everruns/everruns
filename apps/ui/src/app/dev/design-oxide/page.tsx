"use client";

import Link from "next/link";
import { useState } from "react";
import { ArrowLeft, Bot, Flame, Check, Loader2, ChevronRight, Play, Square, Settings, Plus, Search, Bell, User, Zap, Database, Cpu, Activity, Code, Layers, GitBranch, Box } from "lucide-react";
import { cn } from "@/lib/utils";

const isDev = process.env.NODE_ENV === "development";

/**
 * Design Concept: "Oxide"
 *
 * Philosophy:
 * - Sharp corners with subtle chamfers
 * - Warm amber/orange accent inspired by Rust, industrial tools
 * - Deep charcoal backgrounds
 * - Technical but approachable
 * - High legibility with warm undertones
 */

// Sharp corner button
function OxideButton({
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
    default: "bg-[hsl(30_8%_18%)] text-[hsl(30_10%_90%)] hover:bg-[hsl(30_8%_22%)] border border-[hsl(30_8%_25%)]",
    accent: "bg-[hsl(25_90%_55%)] text-[hsl(30_8%_8%)] hover:bg-[hsl(25_90%_60%)] font-medium",
    ghost: "text-[hsl(30_10%_60%)] hover:text-[hsl(30_10%_90%)] hover:bg-[hsl(30_8%_15%)]",
    outline: "border border-[hsl(25_90%_55%/_0.4)] text-[hsl(25_90%_65%)] hover:bg-[hsl(25_90%_55%/_0.1)]",
  };

  const sizes = {
    default: "px-4 py-2 text-sm",
    sm: "px-3 py-1.5 text-xs",
    icon: "p-2",
  };

  return (
    <button className={cn(
      "inline-flex items-center justify-center gap-2 rounded-sm transition-colors",
      variants[variant],
      sizes[size],
      className
    )}>
      {children}
    </button>
  );
}

// Card with subtle corner
function OxideCard({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={cn(
      "rounded-sm border border-[hsl(30_8%_20%)] bg-[hsl(30_8%_12%)]",
      className
    )}>
      {children}
    </div>
  );
}

// Input
function OxideInput({ placeholder, className = "" }: { placeholder?: string; className?: string }) {
  return (
    <input
      type="text"
      placeholder={placeholder}
      className={cn(
        "w-full rounded-sm border border-[hsl(30_8%_20%)] bg-[hsl(30_8%_8%)] px-3 py-2 text-sm",
        "text-[hsl(30_10%_90%)] placeholder:text-[hsl(30_10%_40%)]",
        "focus:outline-none focus:border-[hsl(25_90%_55%)] focus:ring-1 focus:ring-[hsl(25_90%_55%/_0.3)]",
        className
      )}
    />
  );
}

// Badge
function OxideBadge({
  children,
  variant = "default"
}: {
  children: React.ReactNode;
  variant?: "default" | "success" | "warning" | "accent";
}) {
  const variants = {
    default: "bg-[hsl(30_8%_18%)] text-[hsl(30_10%_60%)] border-[hsl(30_8%_25%)]",
    success: "bg-[hsl(145_60%_40%/_0.15)] text-[hsl(145_60%_55%)] border-[hsl(145_60%_40%/_0.3)]",
    warning: "bg-[hsl(45_100%_50%/_0.15)] text-[hsl(45_100%_60%)] border-[hsl(45_100%_50%/_0.3)]",
    accent: "bg-[hsl(25_90%_55%/_0.15)] text-[hsl(25_90%_65%)] border-[hsl(25_90%_55%/_0.3)]",
  };

  return (
    <span className={cn(
      "inline-flex items-center px-2 py-0.5 text-xs font-medium rounded-sm border",
      variants[variant]
    )}>
      {children}
    </span>
  );
}

// Chat components
function OxideUserMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] rounded-sm border border-[hsl(25_90%_55%/_0.3)] bg-[hsl(25_90%_55%/_0.08)] px-3 py-2">
        <p className="text-sm text-[hsl(30_10%_90%)]">{content}</p>
      </div>
    </div>
  );
}

function OxideAgentMessage({ content }: { content: string }) {
  return (
    <div className="flex gap-2">
      <div className="w-5 h-5 rounded-sm bg-[hsl(25_90%_55%/_0.15)] border border-[hsl(25_90%_55%/_0.3)] flex items-center justify-center flex-shrink-0 mt-0.5">
        <Flame className="w-3 h-3 text-[hsl(25_90%_55%)]" />
      </div>
      <p className="text-sm text-[hsl(30_10%_80%)]">{content}</p>
    </div>
  );
}

function OxideToolCall({ name, status, result }: { name: string; status: "done" | "running"; result?: string }) {
  return (
    <div className="ml-7 text-xs">
      <div className="flex items-center gap-2 text-[hsl(30_10%_55%)]">
        {status === "running" ? (
          <Loader2 className="w-3 h-3 animate-spin text-[hsl(25_90%_55%)]" />
        ) : (
          <Check className="w-3 h-3 text-[hsl(145_60%_50%)]" />
        )}
        <span className="font-mono text-[hsl(25_90%_60%)]">{name}</span>
      </div>
      {result && (
        <div className="mt-1 pl-5 text-[hsl(30_10%_45%)] border-l-2 border-[hsl(30_8%_20%)]">
          <span className="text-[hsl(30_10%_35%)]">$</span> {result}
        </div>
      )}
    </div>
  );
}

// Nav item
function NavItem({ icon: Icon, label, active = false }: { icon: React.ElementType; label: string; active?: boolean }) {
  return (
    <div className={cn(
      "flex items-center gap-3 px-3 py-2 text-sm cursor-pointer transition-colors rounded-sm mx-2",
      active
        ? "bg-[hsl(25_90%_55%/_0.12)] text-[hsl(25_90%_65%)]"
        : "text-[hsl(30_10%_55%)] hover:text-[hsl(30_10%_80%)] hover:bg-[hsl(30_8%_15%)]"
    )}>
      <Icon className="w-4 h-4" />
      <span>{label}</span>
    </div>
  );
}

// Stats card
function StatCard({ label, value, icon: Icon, trend }: { label: string; value: string; icon: React.ElementType; trend?: string }) {
  return (
    <OxideCard className="p-4">
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs text-[hsl(30_10%_50%)] uppercase tracking-wide">{label}</span>
        <Icon className="w-4 h-4 text-[hsl(25_90%_55%)]" />
      </div>
      <div className="flex items-baseline gap-2">
        <div className="text-2xl font-semibold text-[hsl(30_10%_90%)]">{value}</div>
        {trend && <span className="text-xs text-[hsl(145_60%_50%)]">{trend}</span>}
      </div>
    </OxideCard>
  );
}

// Agent card
function AgentCard({ name, status, sessions }: { name: string; status: "active" | "idle"; sessions: number }) {
  return (
    <OxideCard className="p-4 hover:border-[hsl(25_90%_55%/_0.3)] transition-colors cursor-pointer">
      <div className="flex items-start justify-between mb-3">
        <div className="flex items-center gap-2">
          <Box className="w-4 h-4 text-[hsl(25_90%_55%)]" />
          <span className="font-medium text-[hsl(30_10%_90%)]">{name}</span>
        </div>
        <OxideBadge variant={status === "active" ? "accent" : "default"}>
          {status}
        </OxideBadge>
      </div>
      <div className="text-xs text-[hsl(30_10%_50%)]">
        {sessions} sessions today
      </div>
    </OxideCard>
  );
}

// Session row
function SessionRow({ id, status, model, time }: { id: string; status: "running" | "completed"; model: string; time: string }) {
  return (
    <div className="flex items-center justify-between px-4 py-3 border-b border-[hsl(30_8%_15%)] hover:bg-[hsl(30_8%_14%)] cursor-pointer transition-colors">
      <div className="flex items-center gap-3">
        {status === "running" ? (
          <div className="w-2 h-2 rounded-full bg-[hsl(25_90%_55%)] animate-pulse" />
        ) : (
          <div className="w-2 h-2 rounded-full bg-[hsl(145_60%_50%)]" />
        )}
        <span className="font-mono text-sm text-[hsl(30_10%_80%)]">{id}</span>
        <span className="text-xs text-[hsl(30_10%_50%)]">{model}</span>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-xs text-[hsl(30_10%_45%)]">{time}</span>
        <ChevronRight className="w-4 h-4 text-[hsl(30_10%_35%)]" />
      </div>
    </div>
  );
}

export default function DesignOxidePage() {
  if (!isDev) {
    return (
      <div className="flex items-center justify-center min-h-screen bg-[hsl(30_8%_8%)]">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-[hsl(30_10%_35%)]">404</h1>
          <p className="text-[hsl(30_10%_35%)] mt-2">Page not found</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[hsl(30_8%_8%)] text-[hsl(30_10%_90%)]">
      <div className="container mx-auto py-8 px-4 max-w-6xl">
        {/* Header */}
        <div className="mb-8">
          <Link
            href="/dev"
            className="inline-flex items-center text-sm text-[hsl(30_10%_55%)] hover:text-[hsl(25_90%_55%)] mb-4 transition-colors"
          >
            <ArrowLeft className="w-4 h-4 mr-2" />
            Back to Developer Tools
          </Link>
          <div className="flex items-center gap-3 mb-2">
            <Flame className="w-6 h-6 text-[hsl(25_90%_55%)]" />
            <h1 className="text-2xl font-bold">Design Concept: Oxide</h1>
          </div>
          <p className="text-[hsl(30_10%_55%)]">
            Sharp corners with subtle radius, warm amber accent, industrial developer aesthetic
          </p>
          <div className="flex gap-2 mt-3">
            <OxideBadge variant="accent">Sharp Corners</OxideBadge>
            <OxideBadge>Amber Accent</OxideBadge>
            <OxideBadge>Warm Tones</OxideBadge>
          </div>
        </div>

        {/* Color Palette */}
        <section className="mb-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(25_90%_55%)] rounded-sm" />
            Color Palette
          </h2>
          <div className="grid grid-cols-6 gap-3">
            {[
              { name: "Accent", color: "hsl(25 90% 55%)", label: "Amber" },
              { name: "Success", color: "hsl(145 60% 45%)", label: "Green" },
              { name: "Warning", color: "hsl(45 100% 50%)", label: "Yellow" },
              { name: "Error", color: "hsl(0 75% 55%)", label: "Red" },
              { name: "Background", color: "hsl(30 8% 8%)", label: "Charcoal" },
              { name: "Card", color: "hsl(30 8% 12%)", label: "Elevated" },
            ].map((c) => (
              <div key={c.name} className="text-center">
                <div
                  className="w-full h-16 rounded-sm border border-[hsl(30_8%_20%)] mb-2"
                  style={{ backgroundColor: c.color }}
                />
                <div className="text-xs font-medium">{c.name}</div>
                <div className="text-xs text-[hsl(30_10%_50%)]">{c.label}</div>
              </div>
            ))}
          </div>
        </section>

        {/* Components Grid */}
        <div className="grid grid-cols-2 gap-6">
          {/* Buttons */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[hsl(25_90%_55%)] rounded-sm" />
              Buttons
            </h2>
            <OxideCard className="p-6 space-y-4">
              <div className="flex gap-3 flex-wrap">
                <OxideButton variant="accent"><Play className="w-4 h-4" /> Run Agent</OxideButton>
                <OxideButton variant="default"><Plus className="w-4 h-4" /> New Session</OxideButton>
                <OxideButton variant="outline"><Settings className="w-4 h-4" /> Configure</OxideButton>
                <OxideButton variant="ghost"><Square className="w-4 h-4" /> Stop</OxideButton>
              </div>
              <div className="flex gap-3">
                <OxideButton variant="accent" size="sm">Small</OxideButton>
                <OxideButton variant="default" size="icon"><Search className="w-4 h-4" /></OxideButton>
                <OxideButton variant="ghost" size="icon"><Bell className="w-4 h-4" /></OxideButton>
              </div>
            </OxideCard>
          </section>

          {/* Inputs */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[hsl(25_90%_55%)] rounded-sm" />
              Inputs
            </h2>
            <OxideCard className="p-6 space-y-4">
              <OxideInput placeholder="Search agents..." />
              <OxideInput placeholder="Enter command..." />
              <div className="flex gap-3">
                <OxideInput placeholder="API Key" className="flex-1" />
                <OxideButton variant="accent">Save</OxideButton>
              </div>
            </OxideCard>
          </section>

          {/* Badges */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[hsl(25_90%_55%)] rounded-sm" />
              Badges & Status
            </h2>
            <OxideCard className="p-6">
              <div className="flex gap-3 flex-wrap mb-4">
                <OxideBadge>default</OxideBadge>
                <OxideBadge variant="accent">running</OxideBadge>
                <OxideBadge variant="success">completed</OxideBadge>
                <OxideBadge variant="warning">pending</OxideBadge>
              </div>
              <div className="flex items-center gap-4 text-xs text-[hsl(30_10%_55%)]">
                <div className="flex items-center gap-2">
                  <div className="w-2 h-2 rounded-full bg-[hsl(25_90%_55%)] animate-pulse" />
                  <span>Running</span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="w-2 h-2 rounded-full bg-[hsl(145_60%_50%)]" />
                  <span>Completed</span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="w-2 h-2 rounded-full bg-[hsl(30_10%_35%)]" />
                  <span>Idle</span>
                </div>
              </div>
            </OxideCard>
          </section>

          {/* Agent Cards */}
          <section>
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <span className="w-1 h-4 bg-[hsl(25_90%_55%)] rounded-sm" />
              Agent Cards
            </h2>
            <div className="grid grid-cols-2 gap-3">
              <AgentCard name="code-reviewer" status="active" sessions={12} />
              <AgentCard name="test-runner" status="idle" sessions={3} />
            </div>
          </section>
        </div>

        {/* Chat Preview */}
        <section className="mt-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(25_90%_55%)] rounded-sm" />
            Chat Interface
          </h2>
          <OxideCard>
            <div className="border-b border-[hsl(30_8%_20%)] px-4 py-3 flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="w-2 h-2 rounded-full bg-[hsl(25_90%_55%)] animate-pulse" />
                <span className="font-mono text-sm">ses_a7f3b2c</span>
                <OxideBadge variant="accent">running</OxideBadge>
              </div>
              <span className="text-xs text-[hsl(30_10%_50%)]">claude-3-5-sonnet</span>
            </div>
            <div className="p-4 space-y-4">
              <OxideUserMessage content="Can you check the test results and fix any failures?" />
              <OxideAgentMessage content="I'll run the test suite and analyze any failures." />
              <OxideToolCall name="bash" status="done" result="running 24 tests... 23 passed, 1 failed" />
              <OxideToolCall name="read_file" status="done" result="src/api/handler.rs:42" />
              <OxideToolCall name="edit_file" status="running" />
              <OxideAgentMessage content="Found the issue - there's a type mismatch on line 42. Fixing now..." />
            </div>
            <div className="border-t border-[hsl(30_8%_20%)] p-3">
              <div className="flex gap-2">
                <OxideInput placeholder="Type a message..." className="flex-1" />
                <OxideButton variant="accent">Send</OxideButton>
              </div>
            </div>
          </OxideCard>
        </section>

        {/* Full Layout Preview */}
        <section className="mt-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(25_90%_55%)] rounded-sm" />
            Full Layout Preview
          </h2>
          <OxideCard className="overflow-hidden">
            <div className="flex h-[500px]">
              {/* Sidebar */}
              <div className="w-56 border-r border-[hsl(30_8%_20%)] bg-[hsl(30_10%_10%)]">
                <div className="p-4 border-b border-[hsl(30_8%_20%)]">
                  <div className="flex items-center gap-2">
                    <div className="w-6 h-6 rounded-sm bg-gradient-to-br from-[hsl(25_90%_55%)] to-[hsl(35_90%_50%)] flex items-center justify-center">
                      <Zap className="w-4 h-4 text-[hsl(30_8%_8%)]" />
                    </div>
                    <span className="font-semibold text-[hsl(30_10%_90%)]">everruns</span>
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
                <div className="h-12 border-b border-[hsl(30_8%_20%)] flex items-center justify-between px-4 bg-[hsl(30_8%_10%)]">
                  <div className="flex items-center gap-3">
                    <OxideInput placeholder="Search..." className="w-64" />
                  </div>
                  <div className="flex items-center gap-2">
                    <OxideButton variant="ghost" size="icon"><Bell className="w-4 h-4" /></OxideButton>
                    <OxideButton variant="ghost" size="icon"><User className="w-4 h-4" /></OxideButton>
                  </div>
                </div>

                {/* Content */}
                <div className="flex-1 p-6 overflow-auto bg-[hsl(30_8%_7%)]">
                  <div className="flex items-center justify-between mb-6">
                    <h3 className="text-xl font-semibold">Agents</h3>
                    <OxideButton variant="accent"><Plus className="w-4 h-4" /> New Agent</OxideButton>
                  </div>

                  {/* Stats row */}
                  <div className="grid grid-cols-4 gap-4 mb-6">
                    <StatCard label="Total Agents" value="12" icon={Cpu} />
                    <StatCard label="Active" value="3" icon={Activity} trend="+2" />
                    <StatCard label="Sessions" value="847" icon={Database} trend="+47" />
                    <StatCard label="Tokens" value="1.2M" icon={Zap} />
                  </div>

                  {/* Session list */}
                  <OxideCard>
                    <div className="px-4 py-3 border-b border-[hsl(30_8%_20%)]">
                      <span className="font-medium text-sm">Recent Sessions</span>
                    </div>
                    <SessionRow id="ses_a7f3b2c" status="running" model="claude-sonnet" time="2m ago" />
                    <SessionRow id="ses_b2e4d1a" status="completed" model="gpt-4o" time="15m ago" />
                    <SessionRow id="ses_c9f8e3b" status="completed" model="claude-sonnet" time="1h ago" />
                  </OxideCard>
                </div>
              </div>
            </div>
          </OxideCard>
        </section>

        {/* Design Notes */}
        <section className="mt-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(25_90%_55%)] rounded-sm" />
            Design Notes
          </h2>
          <OxideCard className="p-6">
            <div className="grid grid-cols-2 gap-6 text-sm">
              <div>
                <h3 className="font-medium text-[hsl(25_90%_60%)] mb-2">Philosophy</h3>
                <ul className="space-y-1 text-[hsl(30_10%_65%)]">
                  <li>- Very small border-radius (2px) for subtle softness</li>
                  <li>- Warm amber accent evokes craftsmanship</li>
                  <li>- Charcoal with warm undertones for comfort</li>
                  <li>- Professional but approachable</li>
                </ul>
              </div>
              <div>
                <h3 className="font-medium text-[hsl(25_90%_60%)] mb-2">Key Changes from Current</h3>
                <ul className="space-y-1 text-[hsl(30_10%_65%)]">
                  <li>- border-radius: 2px (was 0.5rem)</li>
                  <li>- Primary: Amber hsl(25 90% 55%)</li>
                  <li>- Background: warm charcoal hsl(30 8% 8%)</li>
                  <li>- Subtle gradient accents on logo/icons</li>
                </ul>
              </div>
            </div>
          </OxideCard>
        </section>

        {/* Comparison */}
        <section className="mt-10">
          <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
            <span className="w-1 h-4 bg-[hsl(25_90%_55%)] rounded-sm" />
            Oxide vs Terminal Comparison
          </h2>
          <OxideCard className="p-6">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-[hsl(30_10%_50%)]">
                  <th className="pb-3 font-medium">Property</th>
                  <th className="pb-3 font-medium">Terminal (Cyan)</th>
                  <th className="pb-3 font-medium">Oxide (Amber)</th>
                </tr>
              </thead>
              <tbody className="text-[hsl(30_10%_70%)]">
                <tr className="border-t border-[hsl(30_8%_15%)]">
                  <td className="py-2">Border Radius</td>
                  <td className="py-2">0px (sharp)</td>
                  <td className="py-2">2px (subtle)</td>
                </tr>
                <tr className="border-t border-[hsl(30_8%_15%)]">
                  <td className="py-2">Primary Accent</td>
                  <td className="py-2">Cyan hsl(180 70% 45%)</td>
                  <td className="py-2">Amber hsl(25 90% 55%)</td>
                </tr>
                <tr className="border-t border-[hsl(30_8%_15%)]">
                  <td className="py-2">Background</td>
                  <td className="py-2">Cool dark hsl(220 20% 8%)</td>
                  <td className="py-2">Warm dark hsl(30 8% 8%)</td>
                </tr>
                <tr className="border-t border-[hsl(30_8%_15%)]">
                  <td className="py-2">Feel</td>
                  <td className="py-2">Terminal, CLI, hacker</td>
                  <td className="py-2">Industrial, craftsman, Rust</td>
                </tr>
                <tr className="border-t border-[hsl(30_8%_15%)]">
                  <td className="py-2">Status Indicators</td>
                  <td className="py-2">Square</td>
                  <td className="py-2">Circular</td>
                </tr>
              </tbody>
            </table>
          </OxideCard>
        </section>
      </div>
    </div>
  );
}
