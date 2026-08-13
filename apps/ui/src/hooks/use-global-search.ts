/**
 * Aggregates search results from multiple sources for the command palette.
 *
 * Sources:
 * 1. Static navigation pages (filtered by feature flags, no entity fetch)
 * 2. Organizations (client-side filter over authenticated user's memberships)
 * 3. Agents (client-side filter over cached list)
 * 4. Sessions (client-side filter over first page)
 * 5. Harnesses (client-side filter over cached list)
 * 6. Skills (client-side filter over cached list)
 * 7. MCP Servers (client-side filter over cached list)
 * 8. Capabilities (client-side filter over cached list)
 * 9. ID-based lookup (detects prefixed IDs and provides direct navigation)
 * 10. Evals (client-side filter over cached list)
 * 11. Apps (client-side filter over cached list)
 * 12. Agent Identities (client-side filter over cached list)
 * 13. Memories, knowledge indexes, plugins, observers, and saved reports
 *
 * All entity searches are client-side over already-fetched React Query data.
 * Backend search endpoints are available for server-side filtering when needed.
 */
"use client";

import { useMemo } from "react";
import {
  MessageSquare,
  Shield,
  Boxes,
  UserRound,
  Rocket,
  Settings,
  Cog,
  Workflow,
  ListTodo,
  Calendar,
  MessageCircle,
  FlaskConical,
  Server,
  HardDrive,
  Building2,
  ChartColumn,
  Library,
  Telescope,
  WalletCards,
  CircuitBoard,
} from "lucide-react";
import type { IconComponent } from "@/lib/capability-icons";
import {
  registryDomainIcons,
  registryNavigationByHref,
  type RegistryNavigationItem,
} from "@/lib/registry-navigation";
import { useAgents } from "@/hooks/use-agents";
import { useSessions } from "@/hooks/use-sessions";
import { useHarnesses } from "@/hooks/use-harnesses";
import { useSkills } from "@/hooks/use-skills";
import { useMcpServers } from "@/hooks/use-mcp-servers";
import { useCapabilities, useDeclarativeCapabilities } from "@/hooks/use-capabilities";
import { useEvals } from "@/hooks/use-evals";
import { useApps } from "@/hooks/use-apps";
import { useAgentIdentities } from "@/hooks/use-agent-identities";
import { useMemories } from "@/hooks/use-memory";
import { useKnowledgeIndexes } from "@/hooks/use-knowledge-indexes";
import { useInstalledPlugins } from "@/hooks/use-plugins";
import { useObservers } from "@/hooks/use-observers";
import { useSavedReports } from "@/hooks/use-reporting";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { localizedCapabilityName } from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";
import { useOrg } from "@/providers/org-provider";
import { useFeatureFlags } from "@/providers/feature-flags-provider";
import type { FeatureFlags } from "@/lib/api/types";

export type SearchResultCategory =
  | "navigation"
  | "agent"
  | "agent_identity"
  | "session"
  | "harness"
  | "skill"
  | "mcp_server"
  | "capability"
  | "app"
  | "eval"
  | "memory"
  | "knowledge_index"
  | "plugin"
  | "observer"
  | "report"
  | "organization"
  | "id";

export interface SearchResult {
  id: string;
  category: SearchResultCategory;
  icon: IconComponent;
  title: string;
  /** Breadcrumb-style subtitle, e.g. "Agents > Daytona Coder" */
  subtitle?: string;
  href: string;
  onSelect?: () => void;
}

interface NavigationPage {
  title: string;
  href: string;
  icon: IconComponent;
  keywords?: string[];
  flag?: keyof FeatureFlags;
}

function registryPage(item: RegistryNavigationItem, flag?: keyof FeatureFlags): NavigationPage {
  return {
    title: item.name,
    href: item.href,
    icon: item.icon,
    keywords: item.keywords,
    flag,
  };
}

const NAVIGATION_PAGES: NavigationPage[] = [
  {
    title: "Sessions",
    href: "/sessions",
    icon: MessageSquare,
    keywords: ["chat", "conversation"],
  },
  {
    title: "Reports",
    href: "/reports",
    icon: ChartColumn,
    keywords: ["analytics", "saved report"],
  },
  {
    title: "Chats",
    href: "/chats",
    icon: MessageCircle,
    keywords: ["global chat", "thread"],
  },
  {
    title: "Agents",
    href: "/agents",
    icon: Boxes,
    keywords: ["bot", "assistant"],
  },
  {
    title: "Agent Identities",
    href: "/agent-identities",
    icon: UserRound,
    keywords: ["persona", "principal", "identity"],
  },
  {
    title: "Harnesses",
    href: "/harnesses",
    icon: Shield,
    keywords: ["template", "config"],
  },
  registryPage(registryNavigationByHref["/skills"], "skills"),
  {
    title: "Memory",
    href: "/memory",
    icon: HardDrive,
    keywords: ["workspace", "files", "storage"],
    flag: "memory",
  },
  {
    title: "Knowledge Indexes",
    href: "/knowledge-indexes",
    icon: Library,
    keywords: ["knowledge", "index", "search", "retrieval"],
    flag: "knowledge",
  },
  registryPage(registryNavigationByHref["/models"]),
  registryPage(registryNavigationByHref["/capabilities"]),
  registryPage(registryNavigationByHref["/plugins"], "plugins"),
  {
    title: "Apps",
    href: "/apps",
    icon: Rocket,
    keywords: ["deploy", "channel"],
  },
  {
    title: "Evals",
    href: "/evals",
    icon: FlaskConical,
    keywords: ["evaluation", "test", "benchmark", "score"],
    flag: "evals",
  },
  {
    title: "Observers",
    href: "/observers",
    icon: Telescope,
    keywords: ["monitor", "score", "production eval"],
    flag: "observers",
  },
  registryPage(registryNavigationByHref["/mcp-servers"]),
  {
    title: "Settings",
    href: "/settings",
    icon: Settings,
    keywords: ["preferences", "config"],
  },
  {
    title: "Settings > Profile",
    href: "/settings/profile",
    icon: Settings,
    keywords: ["account", "profile"],
  },
  {
    title: "Settings > Personal access tokens",
    href: "/settings/personal-access-tokens",
    icon: Settings,
    keywords: ["token", "key", "api key", "pat", "personal access token"],
  },
  {
    title: "Settings > Connections",
    href: "/settings/connections",
    icon: Settings,
    keywords: ["github", "gitlab"],
  },
  {
    title: "Settings > LLM Providers",
    href: "/settings/providers",
    icon: Settings,
    keywords: ["openai", "anthropic", "credentials"],
  },
  {
    title: "Settings > Organization",
    href: "/settings/organization",
    icon: Settings,
    keywords: [
      "org",
      "organization",
      "organizations",
      "organisation",
      "organisations",
      "team",
      "switch",
    ],
  },
  {
    title: "Settings > Members",
    href: "/settings/members",
    icon: Settings,
    keywords: ["team", "invite"],
  },
  {
    title: "Settings > Features",
    href: "/settings/features",
    icon: Settings,
    keywords: ["feature", "flags", "experimental", "opt-in", "beta"],
  },
  {
    title: "Settings > Payments",
    href: "/settings/payments",
    icon: WalletCards,
    keywords: ["wallet", "spend", "billing"],
    flag: "machine_payments",
  },
  {
    title: "Durable Execution",
    href: "/durable",
    icon: Cog,
    keywords: ["workflow", "worker", "queue"],
  },
  { title: "Durable > Workers", href: "/durable/workers", icon: Server },
  { title: "Durable > Workflows", href: "/durable/workflows", icon: Workflow },
  { title: "Durable > Queues", href: "/durable/queues", icon: ListTodo },
  { title: "Durable > Schedules", href: "/durable/schedules", icon: Calendar },
  {
    title: "Durable > Circuit Breakers",
    href: "/durable/circuit-breakers",
    icon: CircuitBoard,
    keywords: ["failure", "resilience"],
  },
  { title: "Dev Tools", href: "/dev", icon: FlaskConical },
];

/** Known ID prefixes and where they resolve. */
const ID_PREFIX_MAP: Record<
  string,
  {
    category: SearchResultCategory;
    label: string;
    path: string;
    listOnly?: boolean;
    flag?: keyof FeatureFlags;
  }
> = {
  agent_: { category: "agent", label: "Agent", path: "/agents" },
  session_: { category: "session", label: "Session", path: "/sessions" },
  harness_: { category: "harness", label: "Harness", path: "/harnesses" },
  skill_: { category: "skill", label: "Skill", path: "/skills", flag: "skills" },
  mcp_: {
    category: "mcp_server",
    label: "MCP Server",
    path: "/mcp-servers",
    listOnly: true,
  },
  cap_: {
    category: "capability",
    label: "Declarative Capability",
    path: "/capabilities",
    listOnly: true,
  },
  eval_: { category: "eval", label: "Eval", path: "/evals", flag: "evals" },
  app_: { category: "app", label: "App", path: "/apps" },
  mem_: { category: "id", label: "Memory", path: "/memory", flag: "memory" },
  kidx_: {
    category: "knowledge_index",
    label: "Knowledge Index",
    path: "/knowledge-indexes",
    flag: "knowledge",
  },
  plugin_: {
    category: "plugin",
    label: "Plugin",
    path: "/plugins",
    listOnly: true,
    flag: "plugins",
  },
  observer_: {
    category: "observer",
    label: "Observer",
    path: "/observers",
    flag: "observers",
  },
  identity_: {
    category: "agent_identity",
    label: "Agent Identity",
    path: "/agent-identities",
  },
};

/**
 * Tokenized multi-word search. Every word in the query must appear
 * somewhere in the combined searchable text. This means "Daytona Agent"
 * matches an agent named "Daytona Coder" because "daytona" hits the name
 * and "agent" hits the category context passed via `extraContext`.
 */
function matchesTokens(tokens: string[], ...texts: (string | undefined | null)[]): boolean {
  const combined = texts
    .filter(Boolean)
    .map((t) => t!.toLowerCase())
    .join(" ");
  return tokens.every((token) => combined.includes(token));
}

const EMPTY_ARRAY: never[] = [];

export function useGlobalSearch(query: string) {
  const { locale } = useLocale();
  const { currentOrg, organizations, setCurrentOrg } = useOrg();
  const featureFlags = useFeatureFlags();
  const entitySearchEnabled = query.trim().length > 0;
  const skillsEnabled = featureFlags.skills;
  const evalsEnabled = featureFlags.evals;
  const memoryEnabled = featureFlags.memory;
  const knowledgeEnabled = featureFlags.knowledge;
  const pluginsEnabled = featureFlags.plugins;
  const observersEnabled = featureFlags.observers;
  const navigationPages = useMemo(
    () => NAVIGATION_PAGES.filter((page) => !page.flag || featureFlags[page.flag]),
    [featureFlags],
  );
  const { data: agentsData } = useAgents({ enabled: entitySearchEnabled });
  const { data: sessionsData } = useSessions(
    undefined,
    { limit: 100 },
    { enabled: entitySearchEnabled },
  );
  const { data: harnessesData } = useHarnesses({ enabled: entitySearchEnabled });
  const { data: skillsData } = useSkills({ enabled: skillsEnabled && entitySearchEnabled });
  const { data: mcpServersData } = useMcpServers({ enabled: entitySearchEnabled });
  const { data: capabilitiesData } = useCapabilities({ enabled: entitySearchEnabled });
  const { data: declarativeCapabilitiesData } = useDeclarativeCapabilities({
    enabled: entitySearchEnabled,
  });
  const { data: evalsData } = useEvals({ enabled: evalsEnabled && entitySearchEnabled });
  const { data: appsData } = useApps({ enabled: entitySearchEnabled });
  const { data: agentIdentitiesData } = useAgentIdentities({ enabled: entitySearchEnabled });
  const { data: memoriesData } = useMemories({ enabled: memoryEnabled && entitySearchEnabled });
  const { data: knowledgeIndexesData } = useKnowledgeIndexes({
    enabled: knowledgeEnabled && entitySearchEnabled,
  });
  const { data: installedPluginsData } = useInstalledPlugins({
    enabled: pluginsEnabled && entitySearchEnabled,
  });
  const { data: observersData } = useObservers({
    enabled: observersEnabled && entitySearchEnabled,
  });
  const { data: savedReportsData } = useSavedReports(entitySearchEnabled);

  const agents = agentsData ?? EMPTY_ARRAY;
  const sessions = sessionsData?.data ?? EMPTY_ARRAY;
  const harnesses = harnessesData ?? EMPTY_ARRAY;
  const skills = skillsEnabled ? (skillsData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const mcpServers = mcpServersData ?? EMPTY_ARRAY;
  const capabilities = capabilitiesData ?? EMPTY_ARRAY;
  const declarativeCapabilities = declarativeCapabilitiesData ?? EMPTY_ARRAY;
  const evals = evalsEnabled ? (evalsData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const apps = appsData ?? EMPTY_ARRAY;
  const agentIdentities = agentIdentitiesData ?? EMPTY_ARRAY;
  const memories = memoryEnabled ? (memoriesData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const knowledgeIndexes = knowledgeEnabled ? (knowledgeIndexesData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const installedPlugins = pluginsEnabled ? (installedPluginsData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const observers = observersEnabled ? (observersData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const savedReports = savedReportsData ?? EMPTY_ARRAY;

  return useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) {
      // Show top navigation pages when empty
      return navigationPages.slice(0, 6).map(
        (page): SearchResult => ({
          id: `nav:${page.href}`,
          category: "navigation",
          icon: page.icon,
          title: page.title,
          href: page.href,
        }),
      );
    }

    const tokens = q.split(/\s+/).filter(Boolean);
    const results: SearchResult[] = [];
    const MAX_PER_CATEGORY = 5;

    // 1. ID-based lookup — resolve entity name from cached data when possible
    for (const [prefix, meta] of Object.entries(ID_PREFIX_MAP)) {
      if (meta.flag && !featureFlags[meta.flag]) continue;
      if (q.startsWith(prefix) || q.startsWith(prefix.replace("_", ""))) {
        // Normalize: allow "session3242" or "session_3242"
        const idValue = q.startsWith(prefix) ? q : `${prefix}${q.slice(prefix.length - 1)}`;

        // Try to resolve a friendly name from cached data
        let resolvedName: string | undefined;
        if (prefix === "agent_") {
          const a = agents.find((a) => a.id === idValue);
          resolvedName = a ? getDisplayName(a) : undefined;
        } else if (prefix === "session_") {
          const s = sessions.find((s) => s.id === idValue);
          resolvedName = s?.title ?? s?.preview ?? undefined;
        } else if (prefix === "harness_") {
          resolvedName = harnesses.find((h) => h.id === idValue)?.name;
        } else if (prefix === "skill_") {
          resolvedName = skills.find((s) => s.id === idValue)?.name;
        } else if (prefix === "mcp_") {
          resolvedName = mcpServers.find((m) => m.id === idValue)?.name;
        } else if (prefix === "cap_") {
          const c = declarativeCapabilities.find((c) => c.id === idValue);
          resolvedName = c?.display_name ?? c?.name;
        } else if (prefix === "eval_") {
          resolvedName = evals.find((e) => e.id === idValue)?.name;
        } else if (prefix === "app_") {
          resolvedName = apps.find((a) => a.id === idValue)?.name;
        } else if (prefix === "identity_") {
          resolvedName = agentIdentities.find((ai) => ai.id === idValue)?.name;
        } else if (prefix === "mem_") {
          resolvedName = memories.find((memory) => memory.id === idValue)?.name;
        } else if (prefix === "kidx_") {
          resolvedName = knowledgeIndexes.find((index) => index.id === idValue)?.name;
        } else if (prefix === "plugin_") {
          const plugin = installedPlugins.find((candidate) => candidate.id === idValue);
          resolvedName = plugin?.display_name ?? plugin?.name;
        } else if (prefix === "observer_") {
          resolvedName = observers.find((observer) => observer.id === idValue)?.name;
        }

        results.push({
          id: `id:${idValue}`,
          category: "id",
          icon: Boxes,
          title: resolvedName ? `${meta.label}: ${resolvedName}` : `Go to ${meta.label}`,
          subtitle: idValue,
          href: meta.listOnly ? meta.path : `${meta.path}/${idValue}`,
        });
      }
    }

    // 2. Navigation pages
    let navCount = 0;
    for (const page of navigationPages) {
      if (navCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, page.title, ...(page.keywords ?? []))) {
        results.push({
          id: `nav:${page.href}`,
          category: "navigation",
          icon: page.icon,
          title: page.title,
          href: page.href,
        });
        navCount++;
      }
    }

    // 3. Organizations
    let orgCount = 0;
    for (const org of organizations) {
      if (orgCount >= MAX_PER_CATEGORY) break;
      const isCurrent = currentOrg?.public_id === org.public_id;
      if (
        matchesTokens(
          tokens,
          org.name,
          org.public_id,
          org.role,
          "organization organisation org team tenant switch",
        )
      ) {
        results.push({
          id: `organization:${org.public_id}`,
          category: "organization",
          icon: Building2,
          title: org.name,
          subtitle: isCurrent
            ? `Current organization > ${org.public_id}`
            : `Switch organization > ${org.public_id}`,
          href: "/settings/organization",
          onSelect: isCurrent ? undefined : () => setCurrentOrg(org),
        });
        orgCount++;
      }
    }

    // 4. Agents
    let agentCount = 0;
    for (const agent of agents) {
      if (agentCount >= MAX_PER_CATEGORY) break;
      const agentDisplayName = getDisplayName(agent);
      if (
        matchesTokens(tokens, agent.name, agentDisplayName, agent.description, agent.id, "agent")
      ) {
        results.push({
          id: `agent:${agent.id}`,
          category: "agent",
          icon: Boxes,
          title: agentDisplayName,
          subtitle: `Agents > ${agentDisplayName}`,
          href: `/agents/${agent.id}`,
        });
        agentCount++;
      }
    }

    // 5. Sessions
    let sessionCount = 0;
    for (const session of sessions) {
      if (sessionCount >= MAX_PER_CATEGORY) break;
      const title = session.title || session.preview || session.id;
      if (matchesTokens(tokens, title, session.id, session.preview, "session")) {
        results.push({
          id: `session:${session.id}`,
          category: "session",
          icon: MessageSquare,
          title: title,
          subtitle: `Sessions > ${title.length > 40 ? title.slice(0, 40) + "..." : title}`,
          href: `/sessions/${session.id}/transcript`,
        });
        sessionCount++;
      }
    }

    // 6. Harnesses
    let harnessCount = 0;
    for (const harness of harnesses) {
      if (harnessCount >= MAX_PER_CATEGORY) break;
      if (
        matchesTokens(
          tokens,
          harness.name,
          harness.display_name,
          harness.description,
          harness.id,
          "harness",
        )
      ) {
        const harnessDisplayName = getDisplayName(harness);
        results.push({
          id: `harness:${harness.id}`,
          category: "harness",
          icon: Shield,
          title: harnessDisplayName,
          subtitle: `Harnesses > ${harnessDisplayName}`,
          href: `/harnesses/${harness.id}`,
        });
        harnessCount++;
      }
    }

    // 7. Skills
    let skillCount = 0;
    for (const skill of skills) {
      if (skillCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, skill.name, skill.description, skill.id, "skill")) {
        results.push({
          id: `skill:${skill.id}`,
          category: "skill",
          icon: registryDomainIcons.skills,
          title: skill.name,
          subtitle: `Skills > ${skill.name}`,
          href: `/skills/${skill.id}`,
        });
        skillCount++;
      }
    }

    // 8. MCP Servers
    let mcpCount = 0;
    for (const server of mcpServers) {
      if (mcpCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, server.name, server.description, server.id, "mcp server")) {
        results.push({
          id: `mcp:${server.id}`,
          category: "mcp_server",
          icon: registryDomainIcons.mcpServers,
          title: server.name,
          subtitle: `MCP Servers > ${server.name}`,
          href: `/mcp-servers`,
        });
        mcpCount++;
      }
    }

    // 9. Capabilities
    let capabilityCount = 0;
    for (const capability of capabilities) {
      if (capabilityCount >= MAX_PER_CATEGORY) break;
      // Match localized strings from every locale so e.g. Ukrainian queries
      // find capabilities regardless of the active UI locale.
      const localizedTexts = Object.values(capability.localizations ?? {}).flatMap((loc) => [
        loc.name,
        loc.description,
      ]);
      if (
        matchesTokens(
          tokens,
          capability.name,
          capability.description,
          capability.id,
          capability.category,
          "capability",
          ...localizedTexts,
        )
      ) {
        const capabilityName = localizedCapabilityName(capability, locale);
        results.push({
          id: `capability:${capability.id}`,
          category: "capability",
          icon: registryDomainIcons.capabilities,
          title: capabilityName,
          subtitle: `Capabilities > ${capabilityName}`,
          href: `/capabilities/${capability.id}`,
        });
        capabilityCount++;
      }
    }

    // 9b. Declarative capability resources by public ID/display name
    for (const capability of declarativeCapabilities) {
      if (capabilityCount >= MAX_PER_CATEGORY) break;
      if (
        matchesTokens(
          tokens,
          capability.name,
          capability.display_name,
          capability.description,
          capability.id,
          capability.capability_id,
          "declarative capability",
        )
      ) {
        const title = capability.display_name ?? capability.name;
        results.push({
          id: `declarative-capability:${capability.id}`,
          category: "capability",
          icon: registryDomainIcons.capabilities,
          title,
          subtitle: `Capabilities > ${capability.capability_id}`,
          href: "/capabilities",
        });
        capabilityCount++;
      }
    }

    // 10. Evals
    let evalCount = 0;
    for (const ev of evals) {
      if (evalCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, ev.name, ev.description, ev.id, "eval")) {
        results.push({
          id: `eval:${ev.id}`,
          category: "eval",
          icon: FlaskConical,
          title: ev.name,
          subtitle: `Evals > ${ev.name}`,
          href: `/evals/${ev.id}`,
        });
        evalCount++;
      }
    }

    // 11. Apps
    let appCount = 0;
    for (const app of apps) {
      if (appCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, app.name, app.description, app.id, "app")) {
        results.push({
          id: `app:${app.id}`,
          category: "app",
          icon: Rocket,
          title: app.name,
          subtitle: `Apps > ${app.name}`,
          href: `/apps/${app.id}`,
        });
        appCount++;
      }
    }

    // 12. Agent Identities
    let identityCount = 0;
    for (const identity of agentIdentities) {
      if (identityCount >= MAX_PER_CATEGORY) break;
      if (
        matchesTokens(tokens, identity.name, identity.description, identity.id, "agent identity")
      ) {
        results.push({
          id: `identity:${identity.id}`,
          category: "agent_identity",
          icon: UserRound,
          title: identity.name,
          subtitle: `Agent Identities > ${identity.name}`,
          href: `/agent-identities/${identity.id}`,
        });
        identityCount++;
      }
    }

    // 13. Memories
    let memoryCount = 0;
    for (const memory of memories) {
      if (memoryCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, memory.name, memory.description, memory.id, "memory")) {
        results.push({
          id: `memory:${memory.id}`,
          category: "memory",
          icon: HardDrive,
          title: memory.name,
          subtitle: `Memory > ${memory.name}`,
          href: `/memory/${memory.id}`,
        });
        memoryCount++;
      }
    }

    // 14. Knowledge indexes
    let knowledgeIndexCount = 0;
    for (const index of knowledgeIndexes) {
      if (knowledgeIndexCount >= MAX_PER_CATEGORY) break;
      if (
        matchesTokens(tokens, index.name, index.description, index.id, "knowledge index retrieval")
      ) {
        results.push({
          id: `knowledge-index:${index.id}`,
          category: "knowledge_index",
          icon: Library,
          title: index.name,
          subtitle: `Knowledge Indexes > ${index.name}`,
          href: `/knowledge-indexes/${index.id}`,
        });
        knowledgeIndexCount++;
      }
    }

    // 15. Installed plugins
    let pluginCount = 0;
    for (const plugin of installedPlugins) {
      if (pluginCount >= MAX_PER_CATEGORY) break;
      const title = plugin.display_name ?? plugin.name;
      if (matchesTokens(tokens, plugin.name, title, plugin.description, plugin.id, "plugin")) {
        results.push({
          id: `plugin:${plugin.id}`,
          category: "plugin",
          icon: registryDomainIcons.plugins,
          title,
          subtitle: `Plugins > ${title}`,
          href: "/plugins",
        });
        pluginCount++;
      }
    }

    // 16. Observers
    let observerCount = 0;
    for (const observer of observers) {
      if (observerCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, observer.name, observer.description, observer.id, "observer")) {
        results.push({
          id: `observer:${observer.id}`,
          category: "observer",
          icon: Telescope,
          title: observer.name,
          subtitle: `Observers > ${observer.name}`,
          href: `/observers/${observer.id}`,
        });
        observerCount++;
      }
    }

    // 17. Saved reports
    let reportCount = 0;
    for (const report of savedReports) {
      if (reportCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, report.name, report.description, report.id, "saved report")) {
        results.push({
          id: `report:${report.id}`,
          category: "report",
          icon: ChartColumn,
          title: report.name,
          subtitle: `Reports > ${report.name}`,
          href: "/reports",
        });
        reportCount++;
      }
    }

    return results;
  }, [
    query,
    locale,
    featureFlags,
    navigationPages,
    currentOrg?.public_id,
    organizations,
    setCurrentOrg,
    agents,
    sessions,
    harnesses,
    skills,
    mcpServers,
    capabilities,
    declarativeCapabilities,
    evals,
    apps,
    agentIdentities,
    memories,
    knowledgeIndexes,
    installedPlugins,
    observers,
    savedReports,
  ]);
}
