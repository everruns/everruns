"use client";

import * as icons from "@/components/icons/facet-icons";
import { facetIconSvg } from "@/components/icons/facet-icon-data";
import type { FacetIconName } from "@/components/icons/facet-icon-data";
import { DevPageShell } from "../_components/dev-page-shell";

const iconEntries = [
  ["agent", "Agent", icons.AgentIcon],
  ["chat", "Chat", icons.ChatIcon],
  ["playground", "Playground", icons.PlaygroundIcon],
  ["harness", "Harness", icons.HarnessDomainIcon],
  ["virtualUser", "Virtual user", icons.VirtualUserIcon],
  ["session", "Session", icons.SessionIcon],
  ["exposure", "Exposure", icons.ExposureIcon],
  ["models", "Models", icons.ModelsIcon],
  ["skills", "Skills", icons.SkillsIcon],
  ["capabilities", "Capabilities", icons.CapabilitiesIcon],
  ["plugins", "Plugins", icons.PluginsIcon],
  ["knowledge", "Knowledge", icons.KnowledgeIcon],
  ["memory", "Memory", icons.MemoryIcon],
  ["evals", "Evals", icons.EvalsIcon],
  ["observer", "Observer", icons.ObserverIcon],
  ["report", "Report", icons.ReportIcon],
  ["sandbox", "Sandbox fleet", icons.SandboxIcon],
  ["sandboxTemplate", "Sandbox template", icons.SandboxTemplateIcon],
  ["providerAccount", "Provider account", icons.ProviderAccountIcon],
  ["settings", "Settings", icons.SettingsIcon],
  ["durable", "Durable execution", icons.DurableIcon],
  ["worker", "Worker", icons.WorkerIcon],
  ["workflow", "Workflow", icons.WorkflowIcon],
  ["queue", "Queue", icons.QueueIcon],
  ["schedule", "Schedule", icons.ScheduleIcon],
  ["circuitBreaker", "Circuit breaker", icons.CircuitBreakerIcon],
  ["organization", "Organization", icons.OrganizationIcon],
  ["provider", "LLM provider", icons.ProviderIcon],
  ["team", "Team", icons.TeamIcon],
  ["health", "Health", icons.HealthIcon],
  ["features", "Features", icons.FeaturesIcon],
  ["payments", "Payments", icons.PaymentsIcon],
  ["account", "Account", icons.AccountIcon],
  ["agentExperience", "Agent experience", icons.AgentExperienceIcon],
  ["token", "Access token", icons.TokenIcon],
] as const;

function downloadIcon(name: FacetIconName) {
  const url = URL.createObjectURL(new Blob([facetIconSvg(name)], { type: "image/svg+xml" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = `${name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)}.svg`;
  link.click();
  URL.revokeObjectURL(url);
}

export default function IconsPage() {
  return (
    <DevPageShell
      eyebrow="Slate · Iconography"
      title="Facet icon masters"
      description="Original Intent for Agents. Supporting domain glyphs share a 24-unit grid, square caps, mitered joins, and 1.75-unit outlines. Compare at 16, 20, 24, and 32 pixels before changing a master."
    >
      <div className="mb-6 flex items-center gap-6 border border-border bg-card p-5">
        <icons.AgentIcon size={48} className="text-primary" />
        <p className="text-sm text-muted-foreground">
          Agent is the solid anchor. Icons inherit color from their surface; official integration
          logos and conventional action glyphs retain their identity.
        </p>
      </div>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {iconEntries.map(([name, label, Icon]) => (
          <section key={name} className="border border-border bg-card p-4">
            <div className="mb-4 flex items-center justify-between gap-2">
              <h2 className="text-sm font-medium">{label}</h2>
              <button
                type="button"
                onClick={() => downloadIcon(name)}
                className="text-xs text-muted-foreground hover:text-foreground"
                aria-label={`Download ${label} SVG`}
              >
                SVG ↓
              </button>
            </div>
            <div className="flex h-12 items-end justify-between gap-3 text-foreground">
              {[16, 20, 24, 32].map((size) => (
                <div key={size} className="flex flex-col items-center gap-2">
                  <Icon size={size} />
                  <span className="text-[10px] text-muted-foreground">{size}px</span>
                </div>
              ))}
            </div>
          </section>
        ))}
      </div>
    </DevPageShell>
  );
}
