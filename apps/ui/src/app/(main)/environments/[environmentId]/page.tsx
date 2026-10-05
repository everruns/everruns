"use client";

import { useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { Archive, Container, Loader2 } from "lucide-react";
import { EnvironmentProfilesEditor } from "@/components/agents/environment-profiles-editor";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { PageBreadcrumb, PageContainer, PageMain, PageMasthead } from "@/components/layout";
import { useArchiveEnvironment, useEnvironment, usePageTitle, useReviseEnvironment } from "@/hooks";
import type { Environment, EnvironmentSet } from "@/lib/api/types";

export default function EnvironmentPage() {
  const { environmentId } = useParams<{ environmentId: string }>();
  const { data: environment, isLoading } = useEnvironment(environmentId);
  usePageTitle(environment?.display_name || "Environment", "Environments");
  if (isLoading || !environment) return null;
  return (
    <EnvironmentEditor
      key={`${environment.id}:${environment.current_revision.id}`}
      environment={environment}
    />
  );
}

function EnvironmentEditor({ environment }: { environment: Environment }) {
  const router = useRouter();
  const revise = useReviseEnvironment();
  const archive = useArchiveEnvironment();
  const [displayName, setDisplayName] = useState(environment.display_name);
  const [description, setDescription] = useState(environment.description || "");
  const [configuration, setConfiguration] = useState<EnvironmentSet | null>(() => ({
    policy: "fixed",
    default: environment.name,
    profiles: { [environment.name]: environment.current_revision.profile },
  }));
  const profile = configuration?.profiles?.[configuration.default];
  const save = async () => {
    if (!profile) return;
    await revise.mutateAsync({
      id: environment.id,
      display_name: displayName,
      description: description || null,
      profile,
    });
  };
  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Environments", href: "/environments" },
          { label: environment.display_name },
        ]}
      />
      <PageMasthead
        icon={<Container />}
        title={environment.display_name}
        description={`${environment.name} · revision ${environment.current_revision.revision}${environment.is_managed ? " · managed" : ""}`}
      />
      <PageMain className="max-w-3xl space-y-5">
        <Card>
          <CardHeader>
            <CardTitle>Configuration</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="display-name">Display name</Label>
              <Input
                id="display-name"
                value={displayName}
                onChange={(event) => setDisplayName(event.target.value)}
                disabled={environment.is_managed}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Textarea
                id="description"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
                disabled={environment.is_managed}
              />
            </div>
            <EnvironmentProfilesEditor
              value={configuration}
              onChange={setConfiguration}
              disabled={environment.is_managed}
              definitionMode
            />
          </CardContent>
        </Card>
        {!environment.is_managed ? (
          <div className="flex gap-2">
            <Button variant="accent" onClick={save} disabled={!profile || revise.isPending}>
              {revise.isPending ? <Loader2 className="size-4 animate-spin" /> : null}Save new
              revision
            </Button>
            <Button
              variant="outline"
              onClick={async () => {
                await archive.mutateAsync(environment.id);
                router.push("/environments");
              }}
            >
              <Archive className="size-4" />
              Archive
            </Button>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            Managed Environments are versioned by the platform and cannot be edited.
          </p>
        )}
      </PageMain>
    </PageContainer>
  );
}
