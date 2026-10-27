"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { AgentIcon } from "@/components/icons/facet-icons";
import { Suspense, useState, type ReactNode } from "react";
import {
  Activity,
  AlertTriangle,
  Check,
  ChevronDown,
  CirclePlus,
  Clock3,
  Ellipsis,
  Info,
  Layers3,
  LayoutGrid,
  List as ListIcon,
  Play,
  Plus,
  Settings,
  ShieldCheck,
  Sparkles,
  Trash2,
  Upload,
} from "lucide-react";
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import { AgentCard } from "@/components/agents/agent-card";
import { EntityActionsMenu } from "@/components/entity-actions/entity-actions-menu";
import { HarnessCard } from "@/components/harnesses/harness-card";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { CopyButton } from "@/components/ui/copy-button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { EntityCard } from "@/components/ui/entity-card";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import { SearchInput } from "@/components/ui/search-input";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import {
  EmptyState,
  IconTile,
  PageBreadcrumb,
  PageColumns,
  PageContainer,
  PageControlStrip,
  PageFooter,
  PageMain,
  PageMasthead,
  PageRail,
  RailSection,
  SectionTabs,
} from "@/components/layout/page-layout";
import type { Agent, Capability } from "@/lib/api/types";

const sampleCapabilities: Capability[] = [
  {
    id: "current_time",
    name: "Current Time",
    description: "Returns the current time.",
    icon: "clock",
    status: "available",
  },
];

const sampleId = "agent_019fda100f037c008024046d6b3d74c0";
const sampleAgent: Agent = {
  id: sampleId,
  name: "research-assistant",
  display_name: "Research assistant",
  description: "Answers questions using indexed product knowledge.",
  system_prompt: "Answer questions using the available product knowledge.",
  harness_id: "harness_generic",
  default_model_id: "model_gpt_5",
  tags: ["research", "production"],
  capabilities: [{ ref: "current_time", config: {} }],
  status: "active",
  created_at: "2026-08-04T14:30:00Z",
  updated_at: "2026-08-07T16:10:00Z",
  archived_at: null,
  deleted_at: null,
  session_count: 12,
  app_count: 2,
  effective_harness: {
    id: "harness_generic",
    name: "generic",
    display_name: "Generic",
    source: "explicit",
    status: "active",
  },
  channels: [
    { id: "aep_slack", channel_type: "slack", enabled: true, status: "live" },
    { id: "aep_chat", channel_type: "public_chat", enabled: true, status: "live" },
    { id: "aep_hook", channel_type: "webhook", enabled: true, status: "draft" },
    { id: "aep_a2a", channel_type: "a2a", enabled: false, status: "disabled" },
    { id: "aep_api", channel_type: "api", enabled: true, status: "live" },
  ],
};

function ShowcaseSection({
  id,
  title,
  description,
  sources,
  children,
}: {
  id: string;
  title: string;
  description: string;
  sources: string[];
  children: ReactNode;
}) {
  const ui = useUiTranslations();
  return (
    <section id={id} className="scroll-mt-6 border border-border/70 bg-card/90">
      <div className="border-b border-border/70 px-4 py-3">
        <h2 className="text-base font-semibold text-foreground">{title}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{description}</p>
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">
            {ui("Source")}
          </span>
          {sources.map((source) => (
            <code
              key={source}
              className="border border-border/70 bg-background px-1.5 py-0.5 text-[11px] text-muted-foreground"
            >
              {source}
            </code>
          ))}
        </div>
      </div>
      <div className="p-4">{children}</div>
    </section>
  );
}

function SampleIconTile() {
  return <IconTile size="md" icon={<AgentIcon />} />;
}

export default function DevUiComponentsPage() {
  const ui = useUiTranslations();
  const [environment, setEnvironment] = useState("development");
  const [showArchived, setShowArchived] = useState(true);
  const [notifications, setNotifications] = useState(true);
  const [checked, setChecked] = useState(true);
  const [tab, setTab] = useState("overview");
  const [sectionTab, setSectionTab] = useState("all");

  return (
    <DevPageShell
      eyebrow="Slate design system"
      title={ui("Core UI Components")}
      description={ui(
        "Interactive reference for the shared primitives and composition patterns used across Everruns.",
      )}
      widthClassName="max-w-7xl"
    >
      <div className="space-y-6">
        <nav className="flex flex-wrap gap-2" aria-label={ui("Component groups")}>
          {[
            ["layout", "Layout"],
            ["buttons", "Buttons"],
            ["identity", "IDs & badges"],
            ["cards", "Cards"],
            ["menus", "Menus"],
            ["entity-actions", "Entity actions"],
            ["forms", "Forms"],
            ["feedback", "Feedback"],
          ].map(([href, label]) => (
            <a
              key={href}
              href={`#${href}`}
              className={buttonVariants({ variant: "outline", size: "sm" })}
            >
              {label}
            </a>
          ))}
        </nav>

        <ShowcaseSection
          id="layout"
          title={ui("Application page composition")}
          description={ui(
            "A compact rendering of the same components and content hierarchy used by the Agents list page.",
          )}
          sources={[
            "src/app/(main)/agents/page.tsx",
            "src/components/layout/page-layout.tsx",
            "src/components/agents/agent-card.tsx",
          ]}
        >
          <div className="overflow-hidden border bg-background">
            <PageContainer className="gap-4 p-4">
              {/* A gallery sample, not this page's own location — no group prefix. */}
              <PageBreadcrumb group={false} items={[{ label: ui("Agents") }]} />
              <PageMasthead
                icon={<AgentIcon />}
                title={ui("Agents")}
                badges={
                  <Badge variant="outline" className="font-mono">
                    12
                  </Badge>
                }
                description={ui(
                  "Reusable agent definitions — instructions, capabilities, and model.",
                )}
                meta={
                  <>
                    <span>{ui("10 active")}</span>
                    <span>{ui("2 archived")}</span>
                  </>
                }
                actions={
                  <>
                    <Button variant="outline">
                      <Upload /> {ui("Import")}
                    </Button>
                    <Button variant="accent">
                      <Plus /> {ui("New agent")}
                    </Button>
                  </>
                }
              />
              <PageControlStrip className="flex flex-wrap items-center gap-3">
                <SearchInput placeholder={ui("Search agents…")} containerClassName="w-64" />
                <div className="flex-1" />
                <SectionTabs
                  value={sectionTab}
                  onValueChange={setSectionTab}
                  items={[
                    { value: "all", label: ui("All") },
                    { value: "active", label: ui("Active") },
                    { value: "archived", label: ui("Archived") },
                  ]}
                />
                <div className="flex border">
                  <button
                    type="button"
                    aria-label={ui("Grid view")}
                    aria-pressed={true}
                    className="inline-flex size-8 items-center justify-center border-r bg-muted text-foreground"
                  >
                    <LayoutGrid className="size-4" />
                  </button>
                  <button
                    type="button"
                    aria-label={ui("List view")}
                    aria-pressed={false}
                    className="inline-flex size-8 items-center justify-center text-muted-foreground hover:bg-muted/50"
                  >
                    <ListIcon className="size-4" />
                  </button>
                </div>
              </PageControlStrip>
              <PageColumns>
                <PageMain>
                  <div className="grid gap-4">
                    <AgentCard
                      agent={sampleAgent}
                      allCapabilities={sampleCapabilities}
                      showEditButton
                    />
                  </div>
                </PageMain>
                <PageRail>
                  <RailSection label={ui("Status")}>
                    <div className="flex flex-col gap-1.5 text-[13px]">
                      <div className="flex items-center justify-between text-foreground">
                        <span>{ui("Active")}</span>
                        <span className="text-muted-foreground">10</span>
                      </div>
                      <div className="flex items-center justify-between text-muted-foreground">
                        <span>{ui("Archived")}</span>
                        <span>2</span>
                      </div>
                    </div>
                  </RailSection>
                  <RailSection label={ui("Tags")}>
                    <div className="flex flex-wrap gap-1.5">
                      <span className="border bg-muted px-2 py-0.5 text-xs">{ui("research")}</span>
                      <span className="border bg-muted px-2 py-0.5 text-xs">
                        {ui("production")}
                      </span>
                    </div>
                  </RailSection>
                </PageRail>
              </PageColumns>
              <PageFooter>
                <span>{ui("Showing 10 of 12 agents")}</span>
                <span className="text-primary">{ui("View archived →")}</span>
              </PageFooter>
            </PageContainer>
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          id="buttons"
          title={ui("Buttons")}
          description={ui(
            "All semantic variants, supported sizes, icon buttons, and disabled states.",
          )}
          sources={["src/components/ui/button.tsx", "src/components/ui/tooltip.tsx"]}
        >
          <div className="space-y-5">
            <div className="flex flex-wrap items-center gap-2">
              <Button>{ui("Default")}</Button>
              <Button variant="secondary">{ui("Secondary")}</Button>
              <Button variant="outline">{ui("Outline")}</Button>
              <Button variant="ghost">{ui("Ghost")}</Button>
              <Button variant="accent">
                <Sparkles /> {ui("Accent")}
              </Button>
              <Button variant="destructive">
                <Trash2 /> {ui("Destructive")}
              </Button>
              <Button variant="link">{ui("Link button")}</Button>
              <Button disabled>{ui("Disabled")}</Button>
            </div>
            <Separator />
            <TooltipProvider>
              <div className="flex flex-wrap items-center gap-2">
                <Button size="sm">
                  <CirclePlus /> {ui("Small")}
                </Button>
                <Button>
                  <CirclePlus /> {ui("Default")}
                </Button>
                <Button size="lg">
                  <CirclePlus /> {ui("Large")}
                </Button>
                {(["icon-sm", "icon", "icon-lg"] as const).map((size) => (
                  <Tooltip key={size}>
                    <TooltipTrigger asChild>
                      <Button
                        size={size}
                        variant="outline"
                        aria-label={ui("Settings, {value0}", { value0: size })}
                      >
                        <Settings />
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>{size}</TooltipContent>
                  </Tooltip>
                ))}
              </div>
            </TooltipProvider>
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          id="identity"
          title={ui("Identifiers and badges")}
          description={ui(
            "Readable entity names with exact-ID copy actions, avatars, status badges, and icon tiles.",
          )}
          sources={[
            "src/components/ui/entity-identity.tsx",
            "src/components/ui/entity-card.tsx",
            "src/components/ui/copy-button.tsx",
            "src/components/ui/badge.tsx",
            "src/components/ui/avatar.tsx",
          ]}
        >
          <div className="grid gap-5 lg:grid-cols-2">
            <div className="space-y-3">
              <Label>{ui("EntityIdentity")}</Label>
              <div className="max-w-md border bg-background p-3">
                <EntityIdentity value={sampleId}>{ui("Research assistant")}</EntityIdentity>
              </div>
              <Label>{ui("Standalone copy-ID button")}</Label>
              <div className="flex items-center gap-2">
                <span className="text-sm text-muted-foreground">{ui("Research assistant")}</span>
                <CopyButton
                  value={sampleId}
                  label={ui("Copy ID: {value0}", { value0: sampleId })}
                  kind="id"
                />
              </div>
            </div>
            <div className="space-y-3">
              <div className="flex flex-wrap items-center gap-2">
                <Badge>{ui("Default")}</Badge>
                <Badge variant="secondary">{ui("Secondary")}</Badge>
                <Badge variant="outline">{ui("Outline")}</Badge>
                <Badge variant="accent">{ui("Experimental")}</Badge>
                <Badge variant="success">
                  <Check /> {ui("Healthy")}
                </Badge>
                <Badge variant="destructive">{ui("Failed")}</Badge>
              </div>
              <div className="flex items-center gap-3">
                <Avatar>
                  <AvatarFallback>{ui("ER")}</AvatarFallback>
                </Avatar>
                <SampleIconTile />
                <IconTile icon={<Layers3 />} />
                <span className="text-sm text-muted-foreground">
                  {ui("Avatar · small tile · masthead tile")}
                </span>
              </div>
            </div>
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          id="cards"
          title={ui("Cards")}
          description={ui(
            "The production AgentCard plus the lower-level card primitives it composes.",
          )}
          sources={[
            "src/components/agents/agent-card.tsx",
            "src/components/ui/card.tsx",
            "src/components/ui/entity-card.tsx",
          ]}
        >
          <div className="grid gap-4 lg:grid-cols-3">
            <div className="space-y-2">
              <p className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">
                {ui("Card primitive")}
              </p>
              <Card>
                <CardHeader>
                  <CardTitle>{ui("Card title")}</CardTitle>
                  <CardDescription>
                    {ui("Header, content, action, and footer slots.")}
                  </CardDescription>
                  <CardAction>
                    <Badge variant="outline">{ui("Draft")}</Badge>
                  </CardAction>
                </CardHeader>
                <CardContent className="text-sm text-muted-foreground">
                  {ui("Flexible content stays deliberately unopinionated.")}
                </CardContent>
                <CardFooter className="border-t text-xs text-muted-foreground">
                  {ui("Updated just now")}
                </CardFooter>
              </Card>
            </div>

            <div className="space-y-2">
              <p className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">
                {ui("Production AgentCard")}
              </p>
              <AgentCard agent={sampleAgent} allCapabilities={sampleCapabilities} showEditButton />
              <AgentCard
                agent={{
                  ...sampleAgent,
                  id: "agent_empty",
                  display_name: "No channels",
                  channels: [],
                }}
                showEditButton
              />
              <AgentCard
                agent={{
                  ...sampleAgent,
                  id: "agent_suspended",
                  display_name: "Suspended channels",
                  exposures_suspended: true,
                }}
              />
            </div>

            <div className="space-y-2">
              <p className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">
                {ui("Production HarnessCard")}
              </p>
              <HarnessCard
                harness={{
                  id: "harness_generic",
                  name: "generic",
                  display_name: "Generic",
                  description: "A general-purpose harness for everyday agents.",
                  parent_harness_id: null,
                  default_model_id: null,
                  tags: ["general"],
                  capabilities: sampleAgent.capabilities,
                  is_built_in: true,
                  status: "active",
                  created_at: sampleAgent.created_at,
                  updated_at: sampleAgent.updated_at,
                  archived_at: null,
                  deleted_at: null,
                  session_count: 12,
                  app_count: 2,
                }}
                allCapabilities={sampleCapabilities}
              />
              <p className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">
                {ui("EntityCard row variant")}
              </p>
              <EntityCard
                variant="row"
                icon={<SampleIconTile />}
                title={ui("Support triage")}
                copyValue="agent_support_triage"
                inlineBadges={<Badge variant="outline">{ui("Idle")}</Badge>}
                headerActions={
                  <Button size="icon-sm" variant="ghost" aria-label={ui("More options")}>
                    <Ellipsis />
                  </Button>
                }
              >
                <p className="mt-1 text-xs text-muted-foreground">{ui("Last run 8 minutes ago")}</p>
              </EntityCard>
              <EntityCard
                variant="row"
                icon={<SampleIconTile />}
                title={ui("Incident responder")}
                inlineBadges={<Badge variant="success">{ui("Running")}</Badge>}
                headerActions={
                  <Button size="sm" variant="outline">
                    {ui("View")}
                  </Button>
                }
              />
            </div>
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          id="menus"
          title={ui("Menus and tabs")}
          description={ui(
            "Dropdown actions, labeled selects, segmented tabs, and section navigation.",
          )}
          sources={[
            "src/components/ui/dropdown-menu.tsx",
            "src/components/ui/select.tsx",
            "src/components/ui/tabs.tsx",
            "src/components/layout/page-layout.tsx",
          ]}
        >
          <div className="space-y-5">
            <div className="flex flex-wrap items-end gap-4">
              <DropdownMenu>
                <DropdownMenuTrigger className={buttonVariants({ variant: "outline" })}>
                  {ui("Actions")} <ChevronDown />
                </DropdownMenuTrigger>
                <DropdownMenuPositioner align="start">
                  <DropdownMenuContent className="w-52">
                    <DropdownMenuGroup>
                      <DropdownMenuLabel>{ui("Agent actions")}</DropdownMenuLabel>
                      <DropdownMenuItem>
                        <Play /> {ui("Start session")}{" "}
                        <DropdownMenuShortcut>⌘↵</DropdownMenuShortcut>
                      </DropdownMenuItem>
                      <DropdownMenuItem>
                        <Settings /> {ui("Configure")}
                      </DropdownMenuItem>
                      <DropdownMenuCheckboxItem
                        checked={showArchived}
                        onCheckedChange={setShowArchived}
                      >
                        {ui("Show archived")}
                      </DropdownMenuCheckboxItem>
                    </DropdownMenuGroup>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem variant="destructive">
                      <Trash2 /> {ui("Delete")}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenuPositioner>
              </DropdownMenu>

              <div className="space-y-1.5">
                <Label htmlFor="showcase-environment">{ui("Environment")}</Label>
                <Select value={environment} onValueChange={setEnvironment}>
                  <SelectTrigger id="showcase-environment" className="w-48">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      <SelectLabel>{ui("Runtime")}</SelectLabel>
                      <SelectItem value="development">{ui("Development")}</SelectItem>
                      <SelectItem value="staging">{ui("Staging")}</SelectItem>
                      <SelectSeparator />
                      <SelectItem value="production">{ui("Production")}</SelectItem>
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </div>
            </div>

            <Tabs value={tab} onValueChange={setTab}>
              <TabsList>
                <TabsTrigger value="overview">{ui("Overview")}</TabsTrigger>
                <TabsTrigger value="activity">{ui("Activity")}</TabsTrigger>
                <TabsTrigger value="disabled" disabled>
                  {ui("Disabled")}
                </TabsTrigger>
              </TabsList>
              <TabsContent
                value="overview"
                className="border bg-background p-3 text-sm text-muted-foreground"
              >
                {ui("Overview panel content")}
              </TabsContent>
              <TabsContent
                value="activity"
                className="border bg-background p-3 text-sm text-muted-foreground"
              >
                {ui("Activity panel content")}
              </TabsContent>
            </Tabs>

            <SectionTabs
              value={sectionTab}
              onValueChange={setSectionTab}
              items={[
                { value: "all", label: ui("All") },
                { value: "active", label: ui("Active"), icon: <Activity className="size-4" /> },
                { value: "scheduled", label: ui("Scheduled"), icon: <Clock3 className="size-4" /> },
              ]}
            />
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          id="entity-actions"
          title={ui("Entity actions menu")}
          description={ui(
            "The one header overflow menu on every entity page: the kind's own actions, then Record (History, Manager notes), then Lifecycle (Archive, Delete). Empty groups drop with their divider; Manager notes is hidden from readers.",
          )}
          sources={[
            "src/components/entity-actions/entity-actions-menu.tsx",
            "src/components/entity-actions/history-sheet.tsx",
            "src/components/entity-actions/manager-notes-sheet.tsx",
          ]}
        >
          <Suspense>
            <div className="flex flex-wrap items-start gap-8">
              {[
                { caption: "Manager, every group", manage: true, lifecycle: true },
                { caption: "Reader, record only", manage: false, lifecycle: false },
              ].map((variant) => (
                <div key={variant.caption} className="space-y-2">
                  <p className="text-xs text-muted-foreground">{variant.caption}</p>
                  <EntityActionsMenu
                    entityRef="agent_019fd9b43fa37512b8f25226b21c2c8b"
                    kind="agent"
                    entityName="Support triage"
                    permissions={{ manage: variant.manage }}
                    actions={
                      variant.lifecycle
                        ? [{ id: "copy", label: ui("Copy"), icon: <Layers3 />, onSelect: () => {} }]
                        : []
                    }
                    archive={variant.lifecycle ? { onSelect: () => {} } : undefined}
                    delete={variant.lifecycle ? { onSelect: () => {} } : undefined}
                  />
                </div>
              ))}
            </div>
          </Suspense>
        </ShowcaseSection>

        <ShowcaseSection
          id="forms"
          title={ui("Form controls")}
          description={ui(
            "Inputs, search, textarea, checkbox, switch, validation, and disabled states.",
          )}
          sources={[
            "src/components/ui/input.tsx",
            "src/components/ui/search-input.tsx",
            "src/components/ui/textarea.tsx",
            "src/components/ui/checkbox.tsx",
            "src/components/ui/switch.tsx",
          ]}
        >
          <div className="grid gap-5 md:grid-cols-2">
            <div className="space-y-4">
              <div className="space-y-1.5">
                <Label htmlFor="showcase-name">{ui("Name")}</Label>
                <Input id="showcase-name" defaultValue="Research assistant" />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="showcase-search">{ui("Search")}</Label>
                <SearchInput id="showcase-search" placeholder={ui("Search agents…")} />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="showcase-error">{ui("Invalid value")}</Label>
                <Input id="showcase-error" defaultValue="Duplicate name" aria-invalid />
                <p className="text-xs text-destructive">{ui("This name is already in use.")}</p>
              </div>
            </div>
            <div className="space-y-4">
              <div className="space-y-1.5">
                <Label htmlFor="showcase-instructions">{ui("Instructions")}</Label>
                <Textarea
                  id="showcase-instructions"
                  defaultValue="Be concise, verify sources, and surface uncertainty."
                />
              </div>
              <div className="flex items-center gap-2">
                <Checkbox id="showcase-checkbox" checked={checked} onCheckedChange={setChecked} />
                <Label htmlFor="showcase-checkbox">{ui("Allow tool calls")}</Label>
              </div>
              <div className="flex items-center justify-between border bg-background p-3">
                <div>
                  <Label htmlFor="showcase-switch">{ui("Notifications")}</Label>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {ui("Notify when a run needs attention.")}
                  </p>
                </div>
                <Switch
                  id="showcase-switch"
                  checked={notifications}
                  onCheckedChange={setNotifications}
                />
              </div>
              <Input value="Disabled input" disabled readOnly />
            </div>
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          id="feedback"
          title={ui("Notices and empty states")}
          description={ui(
            "Reusable inline feedback variants and the canonical zero-state composition.",
          )}
          sources={["src/components/ui/notice.tsx", "src/components/layout/page-layout.tsx"]}
        >
          <div className="grid gap-4 lg:grid-cols-2">
            <div className="space-y-3">
              <Notice icon={<Info />}>
                <NoticeTitle>{ui("Information")}</NoticeTitle>
                <NoticeDescription>
                  {ui("Configuration changes apply to the next session.")}
                </NoticeDescription>
              </Notice>
              <Notice variant="warning" icon={<AlertTriangle />}>
                <NoticeTitle>{ui("Review required")}</NoticeTitle>
                <NoticeDescription>
                  {ui("This agent can call tools with side effects.")}
                </NoticeDescription>
              </Notice>
              <Notice variant="success" icon={<ShieldCheck />}>
                <NoticeTitle>{ui("Connection healthy")}</NoticeTitle>
                <NoticeDescription>
                  {ui("Credentials were verified successfully.")}
                </NoticeDescription>
              </Notice>
              <Notice variant="destructive" icon={<AlertTriangle />}>
                <NoticeTitle>{ui("Run failed")}</NoticeTitle>
                <NoticeDescription>{ui("The provider rejected the request.")}</NoticeDescription>
              </Notice>
            </div>
            <EmptyState
              icon={<AgentIcon />}
              title={ui("No agents yet")}
              description={ui("Create an agent to start a session and see activity here.")}
              action={
                <Button>
                  <CirclePlus /> {ui("Create agent")}
                </Button>
              }
              className="h-full min-h-64 bg-background"
            />
          </div>
        </ShowcaseSection>
      </div>
    </DevPageShell>
  );
}
