import {
  Server,
  Activity,
  Key,
  Users,
  Building2,
  User,
  WalletCards,
  FlaskConical,
} from "lucide-react";
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
        icon: Building2,
        description: "Manage organization defaults and memberships",
      },
      {
        name: "LLM Providers",
        href: "/settings/providers",
        keywords: ["openai", "anthropic", "credentials"],
        icon: Server,
        description: "Manage LLM providers",
      },
      {
        name: "Team members",
        href: "/settings/members",
        keywords: ["team", "invite"],
        icon: Users,
        description: "View and manage team members",
      },
      {
        // Admin registry of MCP presets. Shown only to people who can manage it;
        // everyone else adds their own servers in My agent experience.
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
        icon: Activity,
        description: "Review pending integration issues and recovery actions",
      },
      {
        name: "Features",
        href: "/settings/features",
        keywords: ["flags", "experimental", "opt-in", "beta"],
        icon: FlaskConical,
        description: "Enable optional and experimental capabilities",
      },
      {
        name: "Payments",
        href: "/settings/payments",
        keywords: ["wallet", "spend", "billing"],
        flag: "machine_payments",
        icon: WalletCards,
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
        icon: User,
        description: "Manage your Everruns account",
      },
      {
        name: "My agent experience",
        href: "/settings/agent-experience",
        keywords: ["connections", "github", "gitlab", "virtual user", "locale", "timezone"],
        icon: User,
        description: "Your virtual user and connections in this organization",
      },
      {
        name: "Personal access tokens",
        href: "/settings/personal-access-tokens",
        keywords: ["token", "key", "api key", "pat"],
        icon: Key,
        description: "Manage personal access tokens for programmatic access",
      },
    ],
  },
];
