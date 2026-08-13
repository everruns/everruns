"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { Check, UserRound } from "lucide-react";
import { useCreateAgentIdentity } from "@/hooks/use-agent-identities";
import { usePageTitle } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Combobox } from "@/components/ui/combobox";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
} from "@/components/layout";
import { agentIdentityFormSchema, getFieldErrors, type FieldErrors } from "@/lib/form-validation";
import { LOCALE_OPTIONS, TIMEZONE_OPTIONS } from "@/lib/locale-data";

export default function NewAgentIdentityPage() {
  usePageTitle("New Identity", "Agent Identities");
  const router = useRouter();
  const createIdentity = useCreateAgentIdentity();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [locale, setLocale] = useState("");
  const [timezone, setTimezone] = useState("");
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const parsed = agentIdentityFormSchema.safeParse({
      name,
      description,
      locale,
      timezone,
    });
    if (!parsed.success) {
      setFieldErrors(getFieldErrors(parsed.error));
      return;
    }

    const identity = await createIdentity.mutateAsync({
      name: parsed.data.name,
      description: parsed.data.description,
      locale: parsed.data.locale,
      timezone: parsed.data.timezone,
    });
    router.push(`/agent-identities/${identity.id}`);
  }

  const isSaving = createIdentity.isPending;

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Agent Identities", href: "/agent-identities" },
          { label: "New identity" },
        ]}
      />

      <PageMasthead
        icon={<UserRound />}
        title="New identity"
        description="Identities give agents a locale, timezone, and connections to run sessions as."
        actions={
          <>
            <Button type="submit" form="identity-edit-form" disabled={isSaving || !name}>
              <Check className="size-4" />
              {isSaving ? "Creating..." : "Create identity"}
            </Button>
            <Button type="button" variant="outline" onClick={() => router.back()}>
              Discard
            </Button>
          </>
        }
      />

      <form id="identity-edit-form" onSubmit={handleSubmit}>
        <PageColumns>
          <PageMain>
            <Card>
              <CardHeader>
                <CardTitle>Identity Details</CardTitle>
              </CardHeader>
              <CardContent className="space-y-6">
                <div className="space-y-2">
                  <Label htmlFor="name">Name</Label>
                  <Input
                    id="name"
                    value={name}
                    onChange={(e) => {
                      setName(e.target.value);
                      setFieldErrors((prev) => ({ ...prev, name: undefined }));
                    }}
                    aria-invalid={!!fieldErrors.name}
                    required
                  />
                  {fieldErrors.name && (
                    <p className="text-xs text-destructive">{fieldErrors.name}</p>
                  )}
                </div>

                <div className="space-y-2">
                  <Label htmlFor="description">Description</Label>
                  <Textarea
                    id="description"
                    value={description}
                    onChange={(e) => {
                      setDescription(e.target.value);
                      setFieldErrors((prev) => ({ ...prev, description: undefined }));
                    }}
                    aria-invalid={!!fieldErrors.description}
                    placeholder="Describe this identity..."
                    rows={3}
                  />
                  {fieldErrors.description && (
                    <p className="text-xs text-destructive">{fieldErrors.description}</p>
                  )}
                  <p className="text-xs text-muted-foreground">Supports Markdown</p>
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                  <div className="space-y-2">
                    <Label>Locale</Label>
                    <Combobox
                      options={LOCALE_OPTIONS}
                      value={locale}
                      onValueChange={(value) => {
                        setLocale(value);
                        setFieldErrors((prev) => ({ ...prev, locale: undefined }));
                      }}
                      placeholder="Select locale..."
                      searchPlaceholder="Search locales..."
                    />
                    {fieldErrors.locale && (
                      <p className="text-xs text-destructive">{fieldErrors.locale}</p>
                    )}
                  </div>
                  <div className="space-y-2">
                    <Label>Timezone</Label>
                    <Combobox
                      options={TIMEZONE_OPTIONS}
                      value={timezone}
                      onValueChange={(value) => {
                        setTimezone(value);
                        setFieldErrors((prev) => ({ ...prev, timezone: undefined }));
                      }}
                      placeholder="Select timezone..."
                      searchPlaceholder="Search timezones..."
                    />
                    {fieldErrors.timezone && (
                      <p className="text-xs text-destructive">{fieldErrors.timezone}</p>
                    )}
                  </div>
                </div>
              </CardContent>
            </Card>
          </PageMain>

          <PageRail>
            <Card className="border-dashed">
              <CardContent className="pt-6">
                <p className="text-sm text-muted-foreground">
                  An identity bundles a locale, timezone, and connections. After creating it, assign
                  connections so agents can run sessions as this identity.
                </p>
              </CardContent>
            </Card>
          </PageRail>
        </PageColumns>
      </form>

      <PageFooter>
        <BackLink href="/agent-identities">Back to Agent Identities</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
