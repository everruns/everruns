"use client";

import Link from "next/link";
import {
  ArrowRight,
  Blocks,
  Box,
  Check,
  CirclePlus,
  ExternalLink,
  FileCode2,
  Gauge,
  LayoutTemplate,
  Palette,
  Save,
  Type,
} from "lucide-react";
import reference from "../../../../design-reference.json";
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";

const foundations = [
  { name: "Background", token: "--background", className: "bg-background" },
  { name: "Card", token: "--card", className: "bg-card" },
  { name: "Primary", token: "--primary", className: "bg-primary" },
  { name: "Accent", token: "--accent", className: "bg-accent" },
  { name: "Muted", token: "--muted", className: "bg-muted" },
  { name: "Destructive", token: "--destructive", className: "bg-destructive" },
];

const invariants = [
  {
    icon: Box,
    title: "Sharp geometry",
    text: "All component radii are zero. Round forms are reserved for semantic status dots, circular glyphs, and the logo rings.",
  },
  {
    icon: Palette,
    title: "Color has a job",
    text: "Grayscale carries content, navy marks commit actions, and full gold is reserved for the single create action.",
  },
  {
    icon: Type,
    title: "Geist only",
    text: "Geist Sans carries interface copy; Geist Mono carries code, telemetry, identifiers, and compact metadata.",
  },
  {
    icon: Gauge,
    title: "Dense and calm",
    text: "Use the 4px spacing scale, 32px controls, compact labels, flat cards, muted hovers, and restrained motion.",
  },
];

const archetypes = [
  {
    title: "List page",
    zones: ["Breadcrumb", "Masthead + create", "Filters", "Results + facets", "Count footer"],
  },
  {
    title: "Detail page",
    zones: ["Breadcrumb", "Masthead + actions", "Section tabs", "Blocks + sidecar", "Back footer"],
  },
  {
    title: "Edit page",
    zones: [
      "Breadcrumb + Edit",
      "Masthead + consequence",
      "Section tabs",
      "Checks + form",
      "Save + Discard",
    ],
  },
];

const checklist = [
  "Render production components in previews; never recreate lookalikes.",
  "Compose domain cards from shared primitives: AgentCard → EntityCard → Card.",
  "Compose entity screens from the five-zone page-layout primitives.",
  "Use sentence case for headings, actions, menus, and badges.",
  "Use one accent create action or one navy commit action per page—never both.",
  "Use muted hover states, a gold focus ring, 1px borders, and no content-card shadow.",
  "Keep the dot grid on page surfaces and flat card backgrounds on content surfaces.",
  "Keep experimental indication in navigation with the Lucide flask marker.",
];

function ReferenceSection({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section className="border bg-card">
      <div className="border-b px-4 py-3">
        <h2 className="text-base font-semibold text-foreground">{title}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{description}</p>
      </div>
      <div className="p-4">{children}</div>
    </section>
  );
}

export default function DevDesignReferencePage() {
  return (
    <DevPageShell
      eyebrow="Pinned design export"
      title="Slate Reference"
      description="A review surface for the adopted Everruns Design System export and its production implementation contract."
      widthClassName="max-w-7xl"
    >
      <div className="space-y-6">
        <Notice variant="info" icon={<FileCode2 />}>
          <NoticeTitle>{reference.name}</NoticeTitle>
          <NoticeDescription>
            Reviewed {reference.reviewedOn} from <code>{reference.archiveFile}</code>. SHA-256{" "}
            <code className="break-all text-foreground">{reference.archiveSha256}</code>. The export
            is a pinned visual reference; <code>{reference.canonicalRuntime}</code> and{" "}
            <code>{reference.canonicalGuidance}</code> remain canonical.
          </NoticeDescription>
        </Notice>

        <div className="flex flex-wrap gap-2">
          <Link href={reference.componentShowcase}>
            <Button variant="accent">
              <Blocks /> Production components <ArrowRight />
            </Button>
          </Link>
          <a href={reference.sourceUrl} target="_blank" rel="noreferrer">
            <Button variant="outline">
              External design project <ExternalLink />
            </Button>
          </a>
        </div>

        <ReferenceSection
          title="Visual foundations"
          description="These swatches resolve from the runtime CSS variables—not copied values from the export."
        >
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {foundations.map((foundation) => (
              <div
                key={foundation.token}
                className="flex items-center gap-3 border bg-background p-3"
              >
                <span className={`size-10 shrink-0 border ${foundation.className}`} />
                <div className="min-w-0">
                  <p className="text-sm font-medium text-foreground">{foundation.name}</p>
                  <code className="text-xs text-muted-foreground">{foundation.token}</code>
                </div>
              </div>
            ))}
          </div>

          <div className="mt-4 grid gap-3 lg:grid-cols-2">
            <div className="border bg-background p-4">
              <p className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
                Geist Sans
              </p>
              <p className="mt-3 text-2xl font-semibold tracking-[-0.02em] text-foreground">
                Durable agents, calm controls.
              </p>
              <p className="mt-2 text-sm leading-6 text-muted-foreground">
                Compact interface typography with clear hierarchy and direct language.
              </p>
            </div>
            <div className="border bg-background p-4 font-mono">
              <p className="text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
                Geist Mono
              </p>
              <p className="mt-3 text-sm text-foreground">agent_019fda100f037c008024046d6b3d74c0</p>
              <p className="mt-2 text-xs leading-6 text-muted-foreground">
                turn=42 · tools=7 · latency=1.8s
              </p>
            </div>
          </div>
        </ReferenceSection>

        <ReferenceSection
          title="Non-negotiable invariants"
          description="The highest-signal decisions from the export, retained in DESIGN.md."
        >
          <div className="grid gap-3 md:grid-cols-2">
            {invariants.map((item) => (
              <Card key={item.title}>
                <CardHeader>
                  <div className="flex items-center gap-3">
                    <span className="flex size-8 items-center justify-center border bg-accent/20">
                      <item.icon className="size-4" />
                    </span>
                    <CardTitle>{item.title}</CardTitle>
                  </div>
                </CardHeader>
                <CardContent>
                  <CardDescription className="leading-6">{item.text}</CardDescription>
                </CardContent>
              </Card>
            ))}
          </div>
        </ReferenceSection>

        <ReferenceSection
          title="Action hierarchy"
          description="Button variants communicate intent, not decoration."
        >
          <div className="grid gap-4 md:grid-cols-3">
            <div className="space-y-3 border bg-background p-4">
              <Badge variant="accent">Create</Badge>
              <Button variant="accent">
                <CirclePlus /> New agent
              </Button>
              <p className="text-xs leading-5 text-muted-foreground">
                One gold action, last in a read-only page header.
              </p>
            </div>
            <div className="space-y-3 border bg-background p-4">
              <Badge>Commit</Badge>
              <Button>
                <Save /> Save changes
              </Button>
              <p className="text-xs leading-5 text-muted-foreground">
                One navy action on edit surfaces; pair with outline Discard.
              </p>
            </div>
            <div className="space-y-3 border bg-background p-4">
              <Badge variant="outline">Secondary</Badge>
              <Button variant="outline">Export</Button>
              <p className="text-xs leading-5 text-muted-foreground">
                Copy, export, edit, analyze, disable, and discard stay neutral.
              </p>
            </div>
          </div>
        </ReferenceSection>

        <ReferenceSection
          title="Component composition"
          description="Domain components inherit Slate behavior through shared primitives."
        >
          <div className="grid items-stretch gap-2 md:grid-cols-[1fr_auto_1fr_auto_1fr]">
            {[
              ["AgentCard", "src/components/agents/agent-card.tsx"],
              ["EntityCard", "src/components/ui/entity-card.tsx"],
              ["Card", "src/components/ui/card.tsx"],
            ].map(([name, path], index) => (
              <div key={name} className="contents">
                {index > 0 && (
                  <div className="hidden items-center justify-center text-muted-foreground md:flex">
                    <ArrowRight className="size-4" />
                  </div>
                )}
                <div className="border bg-background p-4">
                  <p className="font-semibold text-foreground">{name}</p>
                  <code className="mt-2 block break-all text-xs text-muted-foreground">{path}</code>
                </div>
              </div>
            ))}
          </div>
          <p className="mt-3 text-xs text-muted-foreground">
            Read left-to-right as “composes.” The domain component remains the preview target.
          </p>
        </ReferenceSection>

        <ReferenceSection
          title="Page archetypes"
          description="Entities supply content to shared list, detail, and edit structures."
        >
          <div className="grid gap-3 lg:grid-cols-3">
            {archetypes.map((archetype) => (
              <Card key={archetype.title}>
                <CardHeader>
                  <div className="flex items-center gap-2">
                    <LayoutTemplate className="size-4 text-muted-foreground" />
                    <CardTitle>{archetype.title}</CardTitle>
                  </div>
                </CardHeader>
                <CardContent>
                  <ol className="space-y-2">
                    {archetype.zones.map((zone, index) => (
                      <li
                        key={zone}
                        className="flex items-center gap-2 border bg-background px-2.5 py-2 text-sm"
                      >
                        <span className="font-mono text-[11px] text-muted-foreground">
                          0{index + 1}
                        </span>
                        <span>{zone}</span>
                      </li>
                    ))}
                  </ol>
                </CardContent>
              </Card>
            ))}
          </div>
        </ReferenceSection>

        <ReferenceSection
          title="Review checklist"
          description="Use this during UI implementation and visual review."
        >
          <div className="grid gap-x-6 gap-y-3 md:grid-cols-2">
            {checklist.map((item) => (
              <div key={item} className="flex items-start gap-2 text-sm leading-6">
                <Check className="mt-1 size-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
                <span>{item}</span>
              </div>
            ))}
          </div>
        </ReferenceSection>
      </div>
    </DevPageShell>
  );
}
