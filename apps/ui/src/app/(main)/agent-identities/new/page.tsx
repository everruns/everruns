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
              <Input value={description} onChange={(e) => setDescription(e.target.value)} />
            </div>
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <Label>Locale</Label>
                <Input
                  value={locale}
                  onChange={(e) => setLocale(e.target.value)}
                  placeholder="en-US"
                />
              </div>
              <div className="space-y-2">
                <Label>Timezone</Label>
                <Input
                  value={timezone}
                  onChange={(e) => setTimezone(e.target.value)}
                  placeholder="America/Los_Angeles"
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
