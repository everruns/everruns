"use client";
import Link from "next/link";
import { useOrg } from "@/providers/org-provider";
import { useState } from "react";
import { useRouter } from "next/navigation";
import { PanelsTopLeft, Plus, ArrowUpRight } from "lucide-react";
import { PageContainer, PageBreadcrumb, PageMasthead } from "@/components/layout/page-layout";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { useApplications } from "@/hooks/use-applications";
import { createApplication } from "@/lib/api/applications";
import { useUiTranslations } from "@/providers/locale-provider";
import { usePageTitle } from "@/hooks";
export default function ApplicationsPage() {
  const ui = useUiTranslations();
  const { currentOrg } = useOrg();
  const canManage = currentOrg?.role === "owner" || currentOrg?.role === "admin";
  usePageTitle(ui("Applications"));
  const router = useRouter();
  const [includeArchived, setIncludeArchived] = useState(false);
  const { data, isLoading, error } = useApplications(includeArchived);
  const [creating, setCreating] = useState(false),
    [name, setName] = useState(""),
    [slug, setSlug] = useState(""),
    [slugEdited, setSlugEdited] = useState(false),
    [busy, setBusy] = useState(false),
    [failure, setFailure] = useState("");
  async function create(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setFailure("");
    try {
      const app = await createApplication({
        name,
        slug,
        description: "",
        config: {
          membership: "org_users",
          invites: [],
          show_in_sidebar: false,
          automations: false,
        },
        resources: [],
      });
      router.push(`/applications/${app.id}`);
    } catch (e) {
      setFailure((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Applications") }]} />
      <PageMasthead
        icon={<PanelsTopLeft />}
        title={ui("Applications")}
        description={ui(
          "Publish a place to work: chats, rooms and shared memory with one audience.",
        )}
        actions={
          <Button disabled={!canManage} onClick={() => setCreating(true)}>
            <Plus className="mr-2 size-4" />
            {ui("New application")}
          </Button>
        }
      />
      {creating && (
        <form
          onSubmit={create}
          className="mb-6 grid max-w-xl gap-4 border border-border bg-card p-6"
        >
          <h2 className="font-semibold">{ui("New application")}</h2>
          <label className="grid gap-2 text-sm">
            {ui("Name")}
            <Input
              required
              maxLength={120}
              value={name}
              onChange={(e) => {
                setName(e.target.value);
                if (!slugEdited)
                  setSlug(
                    e.target.value
                      .toLowerCase()
                      .replace(/[^a-z0-9]+/g, "-")
                      .replace(/-$/, ""),
                  );
              }}
            />
          </label>
          <label className="grid gap-2 text-sm">
            {ui("Stable name")}
            <Input
              required
              pattern="[a-z](?:[a-z0-9]|-){1,46}[a-z0-9]"
              value={slug}
              onChange={(e) => {
                setSlug(e.target.value);
                setSlugEdited(true);
              }}
              placeholder="content-studio"
            />
            <span className="text-xs text-muted-foreground">
              {ui("Used in the public address. This name stays reserved and cannot be changed.")}
            </span>
          </label>
          <div className="flex gap-2">
            <Button type="submit" disabled={busy}>
              {ui("Create application")}
            </Button>
            <Button type="button" variant="ghost" onClick={() => setCreating(false)}>
              {ui("Cancel")}
            </Button>
          </div>
        </form>
      )}
      {(failure || error) && (
        <p role="alert" className="mb-4 text-destructive">
          {failure || ui("Could not load applications")}
        </p>
      )}
      <label className="mb-4 flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={includeArchived}
          onChange={(e) => setIncludeArchived(e.target.checked)}
        />
        {ui("Show archived")}
      </label>
      {isLoading ? (
        <p role="status">{ui("Loading...")}</p>
      ) : !data?.length ? (
        <div className="border border-dashed border-border p-12 text-center">
          <PanelsTopLeft className="mx-auto mb-4 size-8 text-muted-foreground" />
          <h2 className="font-medium">{ui("Your first application starts here")}</h2>
          <p className="mt-2 text-sm text-muted-foreground">
            {ui("Bring your agents and resources together for a dedicated audience.")}
          </p>
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {data.map((app) => (
            <Link
              key={app.id}
              href={`/applications/${app.id}`}
              className="group flex flex-col gap-4 border border-border bg-card p-6 transition-colors hover:bg-muted/40"
            >
              <div className="flex items-center gap-3">
                <PanelsTopLeft className="size-5 text-muted-foreground" />
                <h2 className="flex-1 font-semibold">{app.name}</h2>
                <ArrowUpRight className="size-4 text-muted-foreground" />
              </div>
              <p className="min-h-10 text-sm text-muted-foreground">
                {app.description || ui("Configure chats, rooms, memory and audience.")}
              </p>
              <div className="flex items-center justify-between">
                <Badge variant="outline">
                  {ui(
                    app.status === "published"
                      ? "Published"
                      : app.status === "archived"
                        ? "Archived"
                        : "Draft",
                  )}
                </Badge>
                <span className="text-xs text-muted-foreground">{app.slug}</span>
              </div>
            </Link>
          ))}
        </div>
      )}
    </PageContainer>
  );
}
