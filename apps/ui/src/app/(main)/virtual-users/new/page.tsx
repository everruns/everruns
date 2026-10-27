"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import { virtualUserCreateDraftSchema } from "@/components/chat-mode/page-form-schemas";

import { VirtualUserIcon } from "@/components/icons/facet-icons";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { Check } from "lucide-react";
import { useCreateVirtualUser } from "@/hooks/use-virtual-users";
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
import { virtualUserFormSchema, getFieldErrors, type FieldErrors } from "@/lib/form-validation";
import { LOCALE_OPTIONS, TIMEZONE_OPTIONS } from "@/lib/locale-data";

export default function NewVirtualUserPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("New virtual user"), ui("Virtual Users"));
  const router = useRouter();
  const createIdentity = useCreateVirtualUser();
  const [usage, setUsage] = useState<"end_user" | "service">("service");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [locale, setLocale] = useState("");
  const [timezone, setTimezone] = useState("");
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});

  const chatDraft = usePageFormDraft({
    schema: virtualUserCreateDraftSchema,
    values: { usage, name, description, locale, timezone },
    entityType: "virtual_user",
    label: ui("New virtual user"),
    baseline: "new",
    dirty: !!(name || description || locale || timezone || usage !== "service"),
    writable: !createIdentity.isPending,
    apply: (values) => {
      setUsage(values.usage);
      setName(values.name);
      setDescription(values.description);
      setLocale(values.locale);
      setTimezone(values.timezone);
    },
  });

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (chatDraft.blocked) return;
    const parsed = virtualUserFormSchema.safeParse({
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
      usage,
      name: parsed.data.name,
      description: parsed.data.description,
      locale: parsed.data.locale,
      timezone: parsed.data.timezone,
    });
    router.push(`/virtual-users/${identity.id}`);
  }

  const isSaving = createIdentity.isPending;

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: ui("Virtual Users"), href: "/virtual-users" },
          { label: ui("New virtual user") },
        ]}
      />

      <PageMasthead
        icon={<VirtualUserIcon />}
        title={ui("New virtual user")}
        description={ui("Create an end user profile or an agent service account.")}
        actions={
          <>
            <Button type="submit" form="identity-edit-form" disabled={isSaving || !name}>
              <Check className="size-4" />
              {isSaving ? ui("Creating...") : ui("Create virtual user")}
            </Button>
            <Button type="button" variant="outline" onClick={() => router.back()}>
              {ui("Discard")}
            </Button>
          </>
        }
      />

      <form id="identity-edit-form" onSubmit={handleSubmit}>
        {chatDraft.notice}
        <PageColumns>
          <PageMain>
            <Card>
              <CardHeader>
                <CardTitle>{ui("Profile")}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-6">
                <div className="space-y-2">
                  <Label htmlFor="usage">{ui("Usage")}</Label>
                  <select
                    id="usage"
                    className="h-10 w-full border bg-background px-3 text-sm"
                    value={usage}
                    onChange={(e) => setUsage(e.target.value as "end_user" | "service")}
                  >
                    <option value="end_user">{ui("End user")}</option>
                    <option value="service">{ui("Service")}</option>
                  </select>
                  <p className="text-xs text-muted-foreground">
                    {ui("Usage is fixed when the account is created.")}
                  </p>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="name">{ui("Name")}</Label>
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
                  <Label htmlFor="description">{ui("Description")}</Label>
                  <Textarea
                    id="description"
                    value={description}
                    onChange={(e) => {
                      setDescription(e.target.value);
                      setFieldErrors((prev) => ({ ...prev, description: undefined }));
                    }}
                    aria-invalid={!!fieldErrors.description}
                    placeholder={ui("Describe this virtual user...")}
                    rows={3}
                  />
                  {fieldErrors.description && (
                    <p className="text-xs text-destructive">{fieldErrors.description}</p>
                  )}
                  <p className="text-xs text-muted-foreground">{ui("Supports Markdown")}</p>
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                  <div className="space-y-2">
                    <Label>{ui("Locale")}</Label>
                    <Combobox
                      options={LOCALE_OPTIONS}
                      value={locale}
                      onValueChange={(value) => {
                        setLocale(value);
                        setFieldErrors((prev) => ({ ...prev, locale: undefined }));
                      }}
                      placeholder={ui("Select locale...")}
                      searchPlaceholder="Search locales..."
                    />
                    {fieldErrors.locale && (
                      <p className="text-xs text-destructive">{fieldErrors.locale}</p>
                    )}
                  </div>
                  <div className="space-y-2">
                    <Label>{ui("Timezone")}</Label>
                    <Combobox
                      options={TIMEZONE_OPTIONS}
                      value={timezone}
                      onValueChange={(value) => {
                        setTimezone(value);
                        setFieldErrors((prev) => ({ ...prev, timezone: undefined }));
                      }}
                      placeholder={ui("Select timezone...")}
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
                  {ui(
                    "A virtual user has its own profile, runtime defaults, and connections. End users use agents; service accounts let agents act on external services.",
                  )}
                </p>
              </CardContent>
            </Card>
          </PageRail>
        </PageColumns>
      </form>

      <PageFooter>
        <BackLink href="/virtual-users">{ui("Back to Virtual Users")}</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
