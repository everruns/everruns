"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { ArrowLeft } from "lucide-react";
import { useCreateAgentIdentity } from "@/hooks/use-agent-identities";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Combobox } from "@/components/ui/combobox";
import { LOCALE_OPTIONS, TIMEZONE_OPTIONS } from "@/lib/locale-data";

export default function NewAgentIdentityPage() {
  const router = useRouter();
  const createIdentity = useCreateAgentIdentity();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [locale, setLocale] = useState("");
  const [timezone, setTimezone] = useState("");

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const identity = await createIdentity.mutateAsync({
      name,
      description: description || undefined,
      locale: locale || undefined,
      timezone: timezone || undefined,
    });
    router.push(`/agent-identities/${identity.id}`);
  }

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/agent-identities"
        className="mb-6 inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="mr-2 h-4 w-4" />
        Back to Agent Identities
      </Link>
      <Card>
        <CardHeader>
          <CardTitle>Create Agent Identity</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label>Name</Label>
              <Input value={name} onChange={(e) => setName(e.target.value)} required />
            </div>
            <div className="space-y-2">
              <Label>Description</Label>
              <Textarea
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Describe this identity..."
                rows={3}
              />
              <p className="text-xs text-muted-foreground">Supports Markdown</p>
            </div>
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <Label>Locale</Label>
                <Combobox
                  options={LOCALE_OPTIONS}
                  value={locale}
                  onValueChange={setLocale}
                  placeholder="Select locale..."
                  searchPlaceholder="Search locales..."
                />
              </div>
              <div className="space-y-2">
                <Label>Timezone</Label>
                <Combobox
                  options={TIMEZONE_OPTIONS}
                  value={timezone}
                  onValueChange={setTimezone}
                  placeholder="Select timezone..."
                  searchPlaceholder="Search timezones..."
                />
              </div>
            </div>
            <div className="flex gap-3">
              <Button type="submit" disabled={createIdentity.isPending || !name}>
                {createIdentity.isPending ? "Creating..." : "Create Identity"}
              </Button>
              <Button type="button" variant="outline" onClick={() => router.back()}>
                Cancel
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
