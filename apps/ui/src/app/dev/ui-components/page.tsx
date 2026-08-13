"use client";

import { useState, type ReactNode } from "react";
import {
  Activity,
  AlertTriangle,
  Bot,
  Boxes,
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
import type { Agent } from "@/lib/api/types";

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
  capabilities: [],
  status: "active",
  created_at: "2026-08-04T14:30:00Z",
  updated_at: "2026-08-07T16:10:00Z",
  archived_at: null,
  deleted_at: null,
  session_count: 12,
  app_count: 2,
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
  return (
    <section id={id} className="scroll-mt-6 border border-border/70 bg-card/90">
      <div className="border-b border-border/70 px-4 py-3">
        <h2 className="text-base font-semibold text-foreground">{title}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{description}</p>
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">
            Source
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
  return <IconTile size="md" icon={<Bot />} />;
}

export default function DevUiComponentsPage() {
  const [environment, setEnvironment] = useState("development");
  const [showArchived, setShowArchived] = useState(true);
  const [notifications, setNotifications] = useState(true);
  const [checked, setChecked] = useState(true);
  const [tab, setTab] = useState("overview");
  const [sectionTab, setSectionTab] = useState("all");

  return (
    <DevPageShell
      eyebrow="Slate design system"
      title="Core UI Components"
      description="Interactive reference for the shared primitives and composition patterns used across Everruns."
      widthClassName="max-w-7xl"
    >
      <div className="space-y-6">
        <nav className="flex flex-wrap gap-2" aria-label="Component groups">
          {[
            ["layout", "Layout"],
            ["buttons", "Buttons"],
            ["identity", "IDs & badges"],
            ["cards", "Cards"],
            ["menus", "Menus"],
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
          title="Application page composition"
          description="A compact rendering of the same components and content hierarchy used by the Agents list page."
          sources={[
            "src/app/(main)/agents/page.tsx",
            "src/components/layout/page-layout.tsx",
            "src/components/agents/agent-card.tsx",
          ]}
        >
          <div className="overflow-hidden border bg-background">
            <PageContainer className="gap-4 p-4">
              {/* A gallery sample, not this page's own location — no group prefix. */}
              <PageBreadcrumb group={false} items={[{ label: "Agents" }]} />
              <PageMasthead
                icon={<Boxes />}
                title="Agents"
                badges={
                  <Badge variant="outline" className="font-mono">
                    12
                  </Badge>
                }
                description="Reusable agent definitions — instructions, capabilities, and model."
                meta={
                  <>
                    <span>10 active</span>
                    <span>2 archived</span>
                  </>
                }
                actions={
                  <>
                    <Button variant="outline">
                      <Upload /> Import
                    </Button>
                    <Button variant="accent">
                      <Plus /> New agent
                    </Button>
                  </>
                }
              />
              <PageControlStrip className="flex flex-wrap items-center gap-3">
                <SearchInput placeholder="Search agents…" containerClassName="w-64" />
                <div className="flex-1" />
                <SectionTabs
                  value={sectionTab}
                  onValueChange={setSectionTab}
                  items={[
                    { value: "all", label: "All" },
                    { value: "active", label: "Active" },
                    { value: "archived", label: "Archived" },
                  ]}
                />
                <div className="flex border">
                  <button
                    type="button"
                    aria-label="Grid view"
                    aria-pressed={true}
                    className="inline-flex size-8 items-center justify-center border-r bg-muted text-foreground"
                  >
                    <LayoutGrid className="size-4" />
                  </button>
                  <button
                    type="button"
                    aria-label="List view"
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
                    <AgentCard agent={sampleAgent} showEditButton />
                  </div>
                </PageMain>
                <PageRail>
                  <RailSection label="Status">
                    <div className="flex flex-col gap-1.5 text-[13px]">
                      <div className="flex items-center justify-between text-foreground">
                        <span>Active</span>
                        <span className="text-muted-foreground">10</span>
                      </div>
                      <div className="flex items-center justify-between text-muted-foreground">
                        <span>Archived</span>
                        <span>2</span>
                      </div>
                    </div>
                  </RailSection>
                  <RailSection label="Tags">
                    <div className="flex flex-wrap gap-1.5">
                      <span className="border bg-muted px-2 py-0.5 text-xs">research</span>
                      <span className="border bg-muted px-2 py-0.5 text-xs">production</span>
                    </div>
                  </RailSection>
                </PageRail>
              </PageColumns>
              <PageFooter>
                <span>Showing 10 of 12 agents</span>
                <span className="text-primary">View archived →</span>
              </PageFooter>
            </PageContainer>
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          id="buttons"
          title="Buttons"
          description="All semantic variants, supported sizes, icon buttons, and disabled states."
          sources={["src/components/ui/button.tsx", "src/components/ui/tooltip.tsx"]}
        >
          <div className="space-y-5">
            <div className="flex flex-wrap items-center gap-2">
              <Button>Default</Button>
              <Button variant="secondary">Secondary</Button>
              <Button variant="outline">Outline</Button>
              <Button variant="ghost">Ghost</Button>
              <Button variant="accent">
                <Sparkles /> Accent
              </Button>
              <Button variant="destructive">
                <Trash2 /> Destructive
              </Button>
              <Button variant="link">Link button</Button>
              <Button disabled>Disabled</Button>
            </div>
            <Separator />
            <TooltipProvider>
              <div className="flex flex-wrap items-center gap-2">
                <Button size="sm">
                  <CirclePlus /> Small
                </Button>
                <Button>
                  <CirclePlus /> Default
                </Button>
                <Button size="lg">
                  <CirclePlus /> Large
                </Button>
                {(["icon-sm", "icon", "icon-lg"] as const).map((size) => (
                  <Tooltip key={size}>
                    <TooltipTrigger asChild>
                      <Button size={size} variant="outline" aria-label={`Settings, ${size}`}>
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
          title="Identifiers and badges"
          description="Readable entity names with exact-ID copy actions, avatars, status badges, and icon tiles."
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
              <Label>EntityIdentity</Label>
              <div className="max-w-md border bg-background p-3">
                <EntityIdentity value={sampleId}>Research assistant</EntityIdentity>
              </div>
              <Label>Standalone copy-ID button</Label>
              <div className="flex items-center gap-2">
                <span className="text-sm text-muted-foreground">Research assistant</span>
                <CopyButton value={sampleId} label={`Copy ID: ${sampleId}`} kind="id" />
              </div>
            </div>
            <div className="space-y-3">
              <div className="flex flex-wrap items-center gap-2">
                <Badge>Default</Badge>
                <Badge variant="secondary">Secondary</Badge>
                <Badge variant="outline">Outline</Badge>
                <Badge variant="accent">Experimental</Badge>
                <Badge variant="success">
                  <Check /> Healthy
                </Badge>
                <Badge variant="destructive">Failed</Badge>
              </div>
              <div className="flex items-center gap-3">
                <Avatar>
                  <AvatarFallback>ER</AvatarFallback>
                </Avatar>
                <SampleIconTile />
                <IconTile icon={<Layers3 />} />
                <span className="text-sm text-muted-foreground">
                  Avatar · small tile · masthead tile
                </span>
              </div>
            </div>
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          id="cards"
          title="Cards"
          description="The production AgentCard plus the lower-level card primitives it composes."
          sources={[
            "src/components/agents/agent-card.tsx",
            "src/components/ui/card.tsx",
            "src/components/ui/entity-card.tsx",
          ]}
        >
          <div className="grid gap-4 lg:grid-cols-3">
            <div className="space-y-2">
              <p className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">
                Card primitive
              </p>
              <Card>
                <CardHeader>
                  <CardTitle>Card title</CardTitle>
                  <CardDescription>Header, content, action, and footer slots.</CardDescription>
                  <CardAction>
                    <Badge variant="outline">Draft</Badge>
                  </CardAction>
                </CardHeader>
                <CardContent className="text-sm text-muted-foreground">
                  Flexible content stays deliberately unopinionated.
                </CardContent>
                <CardFooter className="border-t text-xs text-muted-foreground">
                  Updated just now
                </CardFooter>
              </Card>
            </div>

            <div className="space-y-2">
              <p className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">
                Production AgentCard
              </p>
              <AgentCard agent={sampleAgent} showEditButton />
            </div>

            <div className="space-y-2">
              <p className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">
                EntityCard row variant
              </p>
              <EntityCard
                variant="row"
                icon={<SampleIconTile />}
                title="Support triage"
                copyValue="agent_support_triage"
                inlineBadges={<Badge variant="outline">Idle</Badge>}
                headerActions={
                  <Button size="icon-sm" variant="ghost" aria-label="More options">
                    <Ellipsis />
                  </Button>
                }
              >
                <p className="mt-1 text-xs text-muted-foreground">Last run 8 minutes ago</p>
              </EntityCard>
              <EntityCard
                variant="row"
                icon={<SampleIconTile />}
                title="Incident responder"
                inlineBadges={<Badge variant="success">Running</Badge>}
                headerActions={
                  <Button size="sm" variant="outline">
                    View
                  </Button>
                }
              />
            </div>
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          id="menus"
          title="Menus and tabs"
          description="Dropdown actions, labeled selects, segmented tabs, and section navigation."
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
                  Actions <ChevronDown />
                </DropdownMenuTrigger>
                <DropdownMenuPositioner align="start">
                  <DropdownMenuContent className="w-52">
                    <DropdownMenuGroup>
                      <DropdownMenuLabel>Agent actions</DropdownMenuLabel>
                      <DropdownMenuItem>
                        <Play /> Start session <DropdownMenuShortcut>⌘↵</DropdownMenuShortcut>
                      </DropdownMenuItem>
                      <DropdownMenuItem>
                        <Settings /> Configure
                      </DropdownMenuItem>
                      <DropdownMenuCheckboxItem
                        checked={showArchived}
                        onCheckedChange={setShowArchived}
                      >
                        Show archived
                      </DropdownMenuCheckboxItem>
                    </DropdownMenuGroup>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem variant="destructive">
                      <Trash2 /> Delete
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenuPositioner>
              </DropdownMenu>

              <div className="space-y-1.5">
                <Label htmlFor="showcase-environment">Environment</Label>
                <Select value={environment} onValueChange={setEnvironment}>
                  <SelectTrigger id="showcase-environment" className="w-48">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      <SelectLabel>Runtime</SelectLabel>
                      <SelectItem value="development">Development</SelectItem>
                      <SelectItem value="staging">Staging</SelectItem>
                      <SelectSeparator />
                      <SelectItem value="production">Production</SelectItem>
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </div>
            </div>

            <Tabs value={tab} onValueChange={setTab}>
              <TabsList>
                <TabsTrigger value="overview">Overview</TabsTrigger>
                <TabsTrigger value="activity">Activity</TabsTrigger>
                <TabsTrigger value="disabled" disabled>
                  Disabled
                </TabsTrigger>
              </TabsList>
              <TabsContent
                value="overview"
                className="border bg-background p-3 text-sm text-muted-foreground"
              >
                Overview panel content
              </TabsContent>
              <TabsContent
                value="activity"
                className="border bg-background p-3 text-sm text-muted-foreground"
              >
                Activity panel content
              </TabsContent>
            </Tabs>

            <SectionTabs
              value={sectionTab}
              onValueChange={setSectionTab}
              items={[
                { value: "all", label: "All" },
                { value: "active", label: "Active", icon: <Activity className="size-4" /> },
                { value: "scheduled", label: "Scheduled", icon: <Clock3 className="size-4" /> },
              ]}
            />
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          id="forms"
          title="Form controls"
          description="Inputs, search, textarea, checkbox, switch, validation, and disabled states."
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
                <Label htmlFor="showcase-name">Name</Label>
                <Input id="showcase-name" defaultValue="Research assistant" />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="showcase-search">Search</Label>
                <SearchInput id="showcase-search" placeholder="Search agents…" />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="showcase-error">Invalid value</Label>
                <Input id="showcase-error" defaultValue="Duplicate name" aria-invalid />
                <p className="text-xs text-destructive">This name is already in use.</p>
              </div>
            </div>
            <div className="space-y-4">
              <div className="space-y-1.5">
                <Label htmlFor="showcase-instructions">Instructions</Label>
                <Textarea
                  id="showcase-instructions"
                  defaultValue="Be concise, verify sources, and surface uncertainty."
                />
              </div>
              <div className="flex items-center gap-2">
                <Checkbox id="showcase-checkbox" checked={checked} onCheckedChange={setChecked} />
                <Label htmlFor="showcase-checkbox">Allow tool calls</Label>
              </div>
              <div className="flex items-center justify-between border bg-background p-3">
                <div>
                  <Label htmlFor="showcase-switch">Notifications</Label>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Notify when a run needs attention.
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
          title="Notices and empty states"
          description="Reusable inline feedback variants and the canonical zero-state composition."
          sources={["src/components/ui/notice.tsx", "src/components/layout/page-layout.tsx"]}
        >
          <div className="grid gap-4 lg:grid-cols-2">
            <div className="space-y-3">
              <Notice icon={<Info />}>
                <NoticeTitle>Information</NoticeTitle>
                <NoticeDescription>
                  Configuration changes apply to the next session.
                </NoticeDescription>
              </Notice>
              <Notice variant="warning" icon={<AlertTriangle />}>
                <NoticeTitle>Review required</NoticeTitle>
                <NoticeDescription>This agent can call tools with side effects.</NoticeDescription>
              </Notice>
              <Notice variant="success" icon={<ShieldCheck />}>
                <NoticeTitle>Connection healthy</NoticeTitle>
                <NoticeDescription>Credentials were verified successfully.</NoticeDescription>
              </Notice>
              <Notice variant="destructive" icon={<AlertTriangle />}>
                <NoticeTitle>Run failed</NoticeTitle>
                <NoticeDescription>The provider rejected the request.</NoticeDescription>
              </Notice>
            </div>
            <EmptyState
              icon={<Bot />}
              title="No agents yet"
              description="Create an agent to start a session and see activity here."
              action={
                <Button>
                  <CirclePlus /> Create agent
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
