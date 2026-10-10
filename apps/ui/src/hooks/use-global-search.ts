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
 * 11. Virtual Users (client-side filter over cached list)
 * 12. Memories, knowledge indexes, plugins, observers, and saved reports
 *
 * All entity searches are client-side over already-fetched React Query data.
 * Backend search endpoints are available for server-side filtering when needed.
 */
"use client";

import {
  AgentIcon,
  EvalsIcon,
  HarnessDomainIcon,
  KnowledgeIcon,
  MemoryIcon,
  ObserverIcon,
  OrganizationIcon,
  ReportIcon,
  SessionIcon,
  VirtualUserIcon,
} from "@/components/icons/facet-icons";
import { useMemo } from "react";

import type { IconComponent } from "@/lib/capability-icons";
import { registryDomainIcons } from "@/lib/registry-navigation";
import { useAgents } from "@/hooks/use-agents";
import { useSessions } from "@/hooks/use-sessions";
import { useHarnesses } from "@/hooks/use-harnesses";
import { useSkills } from "@/hooks/use-skills";
import { useMcpServers } from "@/hooks/use-mcp-servers";
import { useCapabilities, useDeclarativeCapabilities } from "@/hooks/use-capabilities";
import { useEvals } from "@/hooks/use-evals";
import { useVirtualUsers } from "@/hooks/use-virtual-users";
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
import { usePolicies } from "@/hooks/use-policies";
import { useNavigationPolicy } from "@/hooks/use-navigation-policy";
import {
  defaultNavigationSections,
  sideChatNavigation,
  visibleNavigationSections,
  isDurableNavigationSection,
} from "@/lib/navigation";
import { settingsNavigationSections } from "@/lib/settings-navigation";

export type SearchResultCategory =
  | "navigation"
  | "agent"
  | "virtual_user"
  | "session"
  | "harness"
  | "skill"
  | "mcp_server"
  | "capability"
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
  navigationGroup?: string;
}

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
    path: "/settings/mcp-catalog",
    listOnly: true,
  },
  cap_: {
    category: "capability",
    label: "Declarative Capability",
    path: "/capabilities",
    listOnly: true,
  },
  eval_: { category: "eval", label: "Eval", path: "/evals", flag: "evals" },
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
    category: "virtual_user",
    label: "Virtual User",
    path: "/virtual-users",
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
  const { currentOrg, organizations, setCurrentOrg, hasRole } = useOrg();
  const featureFlags = useFeatureFlags();
  const entitySearchEnabled = query.trim().length > 0;
  const skillsEnabled = featureFlags.skills;
  const evalsEnabled = featureFlags.evals;
  const memoryEnabled = featureFlags.memory;
  const knowledgeEnabled = featureFlags.knowledge;
  const pluginsEnabled = featureFlags.plugins;
  const observersEnabled = featureFlags.observers;
  const reportsEnabled = featureFlags.reports;
  const { can } = usePolicies("durable");
  const durableAllowed = can("durable.view");
  const canNavigate = useNavigationPolicy();
  const navigationPages = useMemo(() => {
    const sections = visibleNavigationSections(
      defaultNavigationSections.filter(
        (section) => !isDurableNavigationSection(section) || durableAllowed,
      ),
      featureFlags,
      hasRole,
      process.env.NODE_ENV === "development",
    );
    const settings = visibleNavigationSections(
      settingsNavigationSections,
      featureFlags,
      hasRole,
      process.env.NODE_ENV === "development",
      canNavigate,
    );
    return sections.flatMap((section) => {
      const group = section.label ?? (section.id === "chats" ? "Chat" : "Pages");
      const items =
        section.id === "chats" ? [...section.items, ...sideChatNavigation] : section.items;
      return items.flatMap((item) => {
        const page = { ...item, title: item.name, navigationGroup: group };
        if (item.activePrefix !== "/settings") return [page];
        return [
          {
            ...page,
            navigationGroup: "Settings",
            keywords: [
              ...(page.keywords ?? []),
              ...(settingsNavigationSections
                .flatMap((section) => section.items)
                .find((setting) => setting.href === item.href)?.keywords ?? []),
            ],
          },
          ...settings
            .flatMap((settingsSection) =>
              settingsSection.items.map((setting) => ({
                ...setting,
                title: setting.name,
                navigationGroup: `Settings · ${settingsSection.label}`,
              })),
            )
            .filter((setting) => setting.href !== item.href),
        ];
      });
    });
  }, [featureFlags, hasRole, durableAllowed, canNavigate]);
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
  const { data: virtualUsersData } = useVirtualUsers({ enabled: entitySearchEnabled });
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
  const { data: savedReportsData } = useSavedReports(reportsEnabled && entitySearchEnabled);

  const agents = agentsData ?? EMPTY_ARRAY;
  const sessions = sessionsData?.data ?? EMPTY_ARRAY;
  const harnesses = harnessesData ?? EMPTY_ARRAY;
  const skills = skillsEnabled ? (skillsData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const mcpServers = mcpServersData ?? EMPTY_ARRAY;
  const capabilities = capabilitiesData ?? EMPTY_ARRAY;
  const declarativeCapabilities = declarativeCapabilitiesData ?? EMPTY_ARRAY;
  const evals = evalsEnabled ? (evalsData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const virtualUsers = virtualUsersData ?? EMPTY_ARRAY;
  const memories = memoryEnabled ? (memoriesData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const knowledgeIndexes = knowledgeEnabled ? (knowledgeIndexesData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const installedPlugins = pluginsEnabled ? (installedPluginsData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const observers = observersEnabled ? (observersData ?? EMPTY_ARRAY) : EMPTY_ARRAY;
  const savedReports = reportsEnabled ? (savedReportsData ?? EMPTY_ARRAY) : EMPTY_ARRAY;

  return useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) {
      // Show all available pages in layout order without fetching entities
      return navigationPages.map((page): SearchResult => ({
        id: `nav:${page.href}`,
        category: "navigation",
        icon: page.icon,
        title: page.title,
        href: page.href,
        subtitle: page.navigationGroup,
        navigationGroup: page.navigationGroup,
      }));
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
        } else if (prefix === "identity_") {
          resolvedName = virtualUsers.find((ai) => ai.id === idValue)?.name;
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
          icon:
            [...defaultNavigationSections, ...settingsNavigationSections]
              .flatMap((section) => section.items)
              .find((item) => item.href === meta.path)?.icon ?? AgentIcon,
          title: resolvedName ? `${meta.label}: ${resolvedName}` : `Go to ${meta.label}`,
          subtitle: idValue,
          href: meta.listOnly ? meta.path : `${meta.path}/${idValue}`,
        });
      }
    }

    // 2. Navigation pages
    for (const page of navigationPages) {
      if (
        matchesTokens(
          tokens,
          page.title,
          page.navigationGroup,
          page.description,
          ...(page.keywords ?? []),
        )
      ) {
        results.push({
          id: `nav:${page.href}`,
          category: "navigation",
          icon: page.icon,
          title: page.title,
          subtitle: page.navigationGroup,
          navigationGroup: page.navigationGroup,
          href: page.href,
        });
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
          icon: OrganizationIcon,
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
          icon: AgentIcon,
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
          icon: SessionIcon,
          title: title,
          subtitle: `Sessions > ${title.length > 40 ? title.slice(0, 40) + "..." : title}`,
          href: `/sessions/${session.id}/trace`,
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
          icon: HarnessDomainIcon,
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
          href: "/settings/mcp-catalog",
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
          icon: EvalsIcon,
          title: ev.name,
          subtitle: `Evals > ${ev.name}`,
          href: `/evals/${ev.id}`,
        });
        evalCount++;
      }
    }

    // 11. Virtual Users
    let identityCount = 0;
    for (const identity of virtualUsers) {
      if (identityCount >= MAX_PER_CATEGORY) break;
      if (matchesTokens(tokens, identity.name, identity.description, identity.id, "virtual user")) {
        results.push({
          id: `identity:${identity.id}`,
          category: "virtual_user",
          icon: VirtualUserIcon,
          title: identity.name,
          subtitle: `Virtual Users > ${identity.name}`,
          href: `/virtual-users/${identity.id}`,
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
          icon: MemoryIcon,
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
          icon: KnowledgeIcon,
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
          icon: ObserverIcon,
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
          icon: ReportIcon,
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
    virtualUsers,
    memories,
    knowledgeIndexes,
    installedPlugins,
    observers,
    savedReports,
  ]);
}
