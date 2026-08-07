"use client";

import { useMemo, useState } from "react";
import { CreditCard, Plus, ShieldCheck, Trash2, WalletCards } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { usePageTitle } from "@/hooks";
import { useCurrentUser } from "@/hooks/use-auth";
import { useAgentIdentities } from "@/hooks/use-agent-identities";
import {
  useCreatePaymentAccount,
  useCreatePaymentPolicy,
  useDisablePaymentAccount,
  useDisablePaymentPolicy,
  usePaymentAccounts,
  usePaymentAttempts,
  usePaymentPolicies,
} from "@/hooks/use-payments";
import type { PaymentOwnerType, PaymentRail, PaymentPolicy } from "@/lib/api/types";
import { EntityIdentity } from "@/components/ui/entity-identity";

const railLabels: Record<PaymentRail, string> = {
  mpp_tempo: "MPP / Tempo",
  x402_base: "x402 / Base",
};

const ownerLabels: Record<PaymentOwnerType, string> = {
  user: "User",
  agent_identity: "Agent Identity",
  organization: "Organization",
};

function formatUsd(value?: number | null) {
  if (value === undefined || value === null) return "None";
  return `$${value.toFixed(2)}`;
}

function splitCsv(value: string) {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

export default function PaymentSettingsPage() {
  usePageTitle("Payments", "Settings");
  const { data: currentUser } = useCurrentUser();
  const { data: identities = [] } = useAgentIdentities();
  const { data: accounts = [], isLoading: accountsLoading } = usePaymentAccounts();
  const { data: policies = [] } = usePaymentPolicies();
  const { data: attempts = [] } = usePaymentAttempts({ limit: 25 });
  const createAccount = useCreatePaymentAccount();
  const disableAccount = useDisablePaymentAccount();
  const createPolicy = useCreatePaymentPolicy();
  const disablePolicy = useDisablePaymentPolicy();

  const [ownerType, setOwnerType] = useState<PaymentOwnerType>("user");
  const [ownerId, setOwnerId] = useState("");
  const [rail, setRail] = useState<PaymentRail>("x402_base");
  const [label, setLabel] = useState("");
  const [publicAddress, setPublicAddress] = useState("");
  const [privateKey, setPrivateKey] = useState("");

  const [policyAccountId, setPolicyAccountId] = useState("");
  const [subjectType, setSubjectType] = useState<PaymentPolicy["subject_type"]>("agent_identity");
  const [subjectId, setSubjectId] = useState("");
  const [perRequest, setPerRequest] = useState("0.30");
  const [capabilities, setCapabilities] = useState("parallel");
  const [hosts, setHosts] = useState("parallelmpp.dev");

  const ownerOptions = useMemo(() => {
    if (ownerType === "agent_identity") {
      return identities.map((identity) => ({ id: identity.id, label: identity.name }));
    }
    if (ownerType === "user" && currentUser) {
      return [{ id: currentUser.id, label: currentUser.name || currentUser.email }];
    }
    return [];
  }, [currentUser, identities, ownerType]);

  const effectiveOwnerId = ownerType === "organization" ? "org" : ownerId;

  const handleCreateAccount = async () => {
    await createAccount.mutateAsync({
      owner_type: ownerType,
      owner_id: effectiveOwnerId,
      rail,
      label: label.trim(),
      public_address: publicAddress.trim() || null,
      private_key: privateKey.trim() || undefined,
      metadata: {},
    });
    setLabel("");
    setPublicAddress("");
    setPrivateKey("");
  };

  const handleCreatePolicy = async () => {
    await createPolicy.mutateAsync({
      payment_account_id: policyAccountId,
      subject_type: subjectType,
      subject_id: subjectType === "org" ? "org" : subjectId,
      allowed_capabilities: splitCsv(capabilities),
      allowed_hosts: splitCsv(hosts),
      rail_preference: [rail],
      max_amount_usd_per_request: Number(perRequest),
      max_amount_usd_per_turn: null,
      max_amount_usd_per_day: null,
      require_approval_above_usd: null,
      metadata: {},
    });
  };

  const canCreateAccount =
    label.trim() && effectiveOwnerId && rail && (rail !== "x402_base" || privateKey.trim());
  const canCreatePolicy =
    policyAccountId && (subjectType === "org" || subjectId.trim()) && Number(perRequest) > 0;

  return (
    <div className="space-y-8">
      <section>
        <div className="mb-4 flex items-center justify-between">
          <div>
            <h2 className="text-xl font-semibold">Payment Wallets</h2>
          </div>
          <WalletCards className="h-5 w-5 text-muted-foreground" />
        </div>

        <Card className="p-4 mb-4">
          <div className="grid gap-3 md:grid-cols-6">
            <div className="space-y-1">
              <Label>Owner</Label>
              <Select
                value={ownerType}
                onValueChange={(value) => setOwnerType(value as PaymentOwnerType)}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="user">User</SelectItem>
                  <SelectItem value="agent_identity">Agent Identity</SelectItem>
                  <SelectItem value="organization">Organization</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1">
              <Label>Owner ID</Label>
              {ownerType === "organization" ? (
                <Input value="org" disabled />
              ) : (
                <Select value={ownerId} onValueChange={setOwnerId}>
                  <SelectTrigger className="w-full">
                    <SelectValue placeholder="Select owner" />
                  </SelectTrigger>
                  <SelectContent>
                    {ownerOptions.map((option) => (
                      <SelectItem key={option.id} value={option.id}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
            </div>
            <div className="space-y-1">
              <Label>Rail</Label>
              <Select value={rail} onValueChange={(value) => setRail(value as PaymentRail)}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="mpp_tempo" disabled>
                    MPP / Tempo
                  </SelectItem>
                  <SelectItem value="x402_base">x402 / Base</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1">
              <Label>Label</Label>
              <Input value={label} onChange={(event) => setLabel(event.target.value)} />
            </div>
            <div className="space-y-1">
              <Label>Address</Label>
              <Input
                value={publicAddress}
                onChange={(event) => setPublicAddress(event.target.value)}
              />
            </div>
            <div className="space-y-1">
              <Label>Private Key</Label>
              <Input
                type="password"
                autoComplete="off"
                value={privateKey}
                onChange={(event) => setPrivateKey(event.target.value)}
                disabled={rail !== "x402_base"}
              />
            </div>
          </div>
          <div className="mt-3 flex justify-end">
            <Button
              onClick={handleCreateAccount}
              disabled={!canCreateAccount || createAccount.isPending}
            >
              <Plus className="h-4 w-4 mr-1" />
              Add Wallet
            </Button>
          </div>
        </Card>

        <Card>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Wallet</TableHead>
                <TableHead>Owner</TableHead>
                <TableHead>Rail</TableHead>
                <TableHead>Address</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="w-10" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {accounts.map((account) => (
                <TableRow key={account.id}>
                  <TableCell className="font-medium">
                    <EntityIdentity value={account.id}>{account.label}</EntityIdentity>
                  </TableCell>
                  <TableCell>
                    {ownerLabels[account.owner_type]} · {account.owner_id}
                  </TableCell>
                  <TableCell>{railLabels[account.rail]}</TableCell>
                  <TableCell className="max-w-[220px] truncate">
                    {account.public_address || "None"}
                  </TableCell>
                  <TableCell>
                    <Badge variant={account.status === "active" ? "default" : "secondary"}>
                      {account.status}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-destructive"
                      onClick={() => disableAccount.mutate(account.id)}
                      disabled={account.status === "disabled"}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
              {!accountsLoading && accounts.length === 0 && (
                <TableRow>
                  <TableCell colSpan={6} className="py-8 text-center text-muted-foreground">
                    No payment wallets.
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </Card>
      </section>

      <section>
        <div className="mb-4 flex items-center justify-between">
          <div>
            <h2 className="text-xl font-semibold">Spend Policies</h2>
          </div>
          <ShieldCheck className="h-5 w-5 text-muted-foreground" />
        </div>

        <Card className="p-4 mb-4">
          <div className="grid gap-3 md:grid-cols-4">
            <div className="space-y-1">
              <Label>Wallet</Label>
              <Select value={policyAccountId} onValueChange={setPolicyAccountId}>
                <SelectTrigger className="w-full">
                  <SelectValue placeholder="Select wallet" />
                </SelectTrigger>
                <SelectContent>
                  {accounts.map((account) => (
                    <SelectItem key={account.id} value={account.id}>
                      {account.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1">
              <Label>Subject</Label>
              <Select
                value={subjectType}
                onValueChange={(value) => setSubjectType(value as PaymentPolicy["subject_type"])}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="agent_identity">Agent Identity</SelectItem>
                  <SelectItem value="user">User</SelectItem>
                  <SelectItem value="agent">Agent</SelectItem>
                  <SelectItem value="app">App</SelectItem>
                  <SelectItem value="session">Session</SelectItem>
                  <SelectItem value="org">Organization</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1">
              <Label>Subject ID</Label>
              <Input
                value={subjectType === "org" ? "org" : subjectId}
                disabled={subjectType === "org"}
                onChange={(event) => setSubjectId(event.target.value)}
              />
            </div>
            <div className="space-y-1">
              <Label>Capabilities</Label>
              <Input
                value={capabilities}
                onChange={(event) => setCapabilities(event.target.value)}
              />
            </div>
            <div className="space-y-1">
              <Label>Hosts</Label>
              <Input value={hosts} onChange={(event) => setHosts(event.target.value)} />
            </div>
            <div className="space-y-1">
              <Label>Per Request</Label>
              <Input value={perRequest} onChange={(event) => setPerRequest(event.target.value)} />
            </div>
          </div>
          <div className="mt-3 flex justify-end">
            <Button
              onClick={handleCreatePolicy}
              disabled={!canCreatePolicy || createPolicy.isPending}
            >
              <Plus className="h-4 w-4 mr-1" />
              Add Policy
            </Button>
          </div>
        </Card>

        <Card>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Subject</TableHead>
                <TableHead>Capabilities</TableHead>
                <TableHead>Hosts</TableHead>
                <TableHead>Per Request</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="w-10" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {policies.map((policy) => (
                <TableRow key={policy.id}>
                  <TableCell>
                    <EntityIdentity value={policy.id}>
                      {policy.subject_type} · {policy.subject_id}
                    </EntityIdentity>
                  </TableCell>
                  <TableCell>{policy.allowed_capabilities.join(", ") || "Any"}</TableCell>
                  <TableCell>{policy.allowed_hosts.join(", ") || "Any"}</TableCell>
                  <TableCell>{formatUsd(policy.max_amount_usd_per_request)}</TableCell>
                  <TableCell>
                    <Badge variant={policy.status === "active" ? "default" : "secondary"}>
                      {policy.status}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-destructive"
                      onClick={() => disablePolicy.mutate(policy.id)}
                      disabled={policy.status === "disabled"}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
              {policies.length === 0 && (
                <TableRow>
                  <TableCell colSpan={6} className="py-8 text-center text-muted-foreground">
                    No spend policies.
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </Card>
      </section>

      <section>
        <div className="mb-4 flex items-center justify-between">
          <div>
            <h2 className="text-xl font-semibold">Recent Attempts</h2>
          </div>
          <CreditCard className="h-5 w-5 text-muted-foreground" />
        </div>
        <Card>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Operation</TableHead>
                <TableHead>Rail</TableHead>
                <TableHead>Amount</TableHead>
                <TableHead>Target</TableHead>
                <TableHead>Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {attempts.map((attempt) => (
                <TableRow key={attempt.id}>
                  <TableCell>
                    {attempt.capability} · {attempt.operation}
                  </TableCell>
                  <TableCell>{attempt.rail ? railLabels[attempt.rail] : "None"}</TableCell>
                  <TableCell>{formatUsd(attempt.amount_usd)}</TableCell>
                  <TableCell className="max-w-[260px] truncate">{attempt.target_url}</TableCell>
                  <TableCell>
                    <Badge variant={attempt.status === "succeeded" ? "default" : "secondary"}>
                      {attempt.status}
                    </Badge>
                  </TableCell>
                </TableRow>
              ))}
              {attempts.length === 0 && (
                <TableRow>
                  <TableCell colSpan={5} className="py-8 text-center text-muted-foreground">
                    No payment attempts.
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </Card>
      </section>
    </div>
  );
}
