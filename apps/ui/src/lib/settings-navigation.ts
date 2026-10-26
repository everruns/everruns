import {
  AccountIcon,
  ChatIcon,
  FeaturesIcon,
  HealthIcon,
  OrganizationIcon,
  PaymentsIcon,
  PluginsIcon,
  ProviderIcon,
  TeamIcon,
  TokenIcon,
  VirtualUserIcon,
} from "@/components/icons/facet-icons";
import { SlackIcon } from "@/components/icons/slack-icon";
import type { NavigationSection } from "@/lib/navigation";
import { registryDomainIcons } from "@/lib/registry-navigation";

// Settings and command search share names, destinations, and feature gates.
export const settingsNavigationSections: NavigationSection[] = [
  {
    label: "Organization",
    items: [
      {
        name: "Organization",
        href: "/settings/organization",
        keywords: [
          "org",
          "organization",
          "organizations",
          "organisation",
          "organisations",
          "team",
          "switch",
        ],
        icon: OrganizationIcon,
        description: "Manage organization defaults and memberships",
      },
      {
        name: "Chat",
        href: "/settings/chat",
        keywords: ["chat", "model", "default", "assistant"],
        icon: ChatIcon,
        description: "Choose the organization model for Everruns Chat",
      },
      {
        name: "Team members",
        href: "/settings/members",
        keywords: ["team", "invite"],
        icon: TeamIcon,
        description: "View and manage team members",
      },
      {
        // Admin registry of MCP presets. Shown only to people who can manage it;
        // everyone else adds their own servers in Settings > MCP servers.
        name: "MCP catalog",
        href: "/settings/mcp-catalog",
        keywords: ["mcp", "mcp servers", "presets", "tool", "integration"],
        icon: registryDomainIcons.mcpServers,
        policy: "mcp_server.manage",
        description: "Manage the MCP presets agents and people can add",
      },
      {
        name: "Slack workspaces",
        href: "/settings/slack",
        icon: SlackIcon,
        description: "Connect Slack workspaces your agents can join",
      },
      {
        name: "Health",
        href: "/settings/health",
        icon: HealthIcon,
        description: "Review pending integration issues and recovery actions",
      },
      {
        name: "Features",
        href: "/settings/features",
        keywords: ["flags", "experimental", "opt-in", "beta"],
        icon: FeaturesIcon,
        description: "Enable optional and experimental capabilities",
      },
      {
        name: "Payments",
        href: "/settings/payments",
        keywords: ["wallet", "spend", "billing"],
        flag: "machine_payments",
        icon: PaymentsIcon,
        description: "Manage payment wallets and spend policies",
      },
    ],
  },
  {
    label: "Personal",
    items: [
      {
        name: "Account",
        href: "/settings/profile",
        keywords: ["account", "profile"],
        icon: AccountIcon,
        description: "Manage your Everruns account",
      },
      {
        name: "Profile",
        href: "/settings/user-profile",
        keywords: ["locale", "timezone", "language", "virtual user", "name"],
        icon: VirtualUserIcon,
        description: "Your name, locale, and timezone in this organization",
      },
      {
        name: "Connections",
        href: "/settings/connections",
        keywords: ["credentials", "secrets", "github", "gitlab", "daytona", "sign-in"],
        icon: ProviderIcon,
        description: "Provider sign-ins and credentials agents use as you",
      },
      {
        name: "MCP servers",
        href: "/settings/mcp-servers",
        keywords: ["mcp", "mcp servers", "tools"],
        icon: registryDomainIcons.mcpServers,
        description: "MCP servers you add and sign in to for yourself",
      },
      {
        name: "AI clients",
        href: "/settings/ai-clients",
        keywords: ["coding agent", "codex", "claude", "cursor", "chatgpt", "connected clients"],
        icon: PluginsIcon,
        description: "AI apps you approved to use Everruns as you",
      },
      {
        name: "Personal access tokens",
        href: "/settings/personal-access-tokens",
        keywords: ["token", "key", "api key", "pat"],
        icon: TokenIcon,
        description: "Manage personal access tokens for programmatic access",
      },
    ],
  },
];
