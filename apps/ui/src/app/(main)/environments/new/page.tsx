"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Container, Loader2 } from "lucide-react";
import { EnvironmentProfilesEditor } from "@/components/agents/environment-profiles-editor";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { PageBreadcrumb, PageContainer, PageMain, PageMasthead } from "@/components/layout";
import { useCreateEnvironment, usePageTitle } from "@/hooks";
import type { EnvironmentSet } from "@/lib/api/types";

export default function NewEnvironmentPage() {
  usePageTitle("New Environment", "Environments");
  const router = useRouter();
  const mutation = useCreateEnvironment();
  const [name, setName] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [description, setDescription] = useState("");
  const [environment, setEnvironment] = useState<EnvironmentSet | null>(null);
  const profile = environment?.profiles?.[environment.default];
  const submit = async () => {
    if (!name || !displayName || !profile) return;
    const created = await mutation.mutateAsync({
      name,
      display_name: displayName,
      ...(description ? { description } : {}),
      profile,
    });
    router.push(`/environments/${created.id}`);
  };
  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: "Environments", href: "/environments" }, { label: "New" }]}
      />
      <PageMasthead
        icon={<Container />}
        title="New Environment"
        description="Create reusable Sandbox configuration. Saving later creates immutable revisions."
      />
      <PageMain className="max-w-3xl space-y-5">
        <Card>
          <CardHeader>
            <CardTitle>Identity</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                value={name}
                onChange={(event) => setName(event.target.value.toLowerCase())}
                placeholder="coding-daytona"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="display-name">Display name</Label>
              <Input
                id="display-name"
                value={displayName}
                onChange={(event) => setDisplayName(event.target.value)}
                placeholder="Coding Daytona"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Textarea
                id="description"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
              />
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>Sandbox configuration</CardTitle>
          </CardHeader>
          <CardContent>
            <EnvironmentProfilesEditor
              value={environment}
              onChange={setEnvironment}
              definitionMode
            />
          </CardContent>
        </Card>
        <Button
          variant="accent"
          onClick={submit}
          disabled={!name || !displayName || !profile || mutation.isPending}
        >
          {mutation.isPending ? <Loader2 className="size-4 animate-spin" /> : null}Create
          Environment
        </Button>
        {mutation.error ? (
          <p className="text-sm text-destructive">{mutation.error.message}</p>
        ) : null}
      </PageMain>
    </PageContainer>
  );
}
