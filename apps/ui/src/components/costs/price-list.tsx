"use client";

// The org's own internal rates. Rates are never edited: a change is a new rate
// from a later date, so past costs keep the rate they used.

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";

import { BASIS_DESCRIPTIONS, BasisBadge, formatUsd } from "@/components/costs/cost-basis";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  createCostRate,
  listCostRates,
  retireCostRate,
  type CostMeter,
  type CostRate,
  type CreateCostRateRequest,
} from "@/lib/api/costs";
import { useUiTranslations } from "@/providers/locale-provider";

const DATE = new Intl.DateTimeFormat("en-US", { year: "numeric", month: "short", day: "numeric" });

export function describeRateMatch(rate: Pick<CostRate, "meter" | "match">): string {
  const parts = Object.entries(rate.match ?? {}).map(([key, value]) =>
    key === "source" ? (value === "managed" ? "built-in models" : "own keys") : `${key} ${value}`,
  );
  const what = rate.meter === "sandbox.time" ? "Sandbox running time" : "Model tokens";
  return parts.length ? `${what} · ${parts.join(" · ")}` : `${what} · everything`;
}

export function describeRatePrice(rate: CostRate): string {
  if (rate.retired) return "Retired";
  if (rate.list_price_multiplier !== undefined && rate.list_price_multiplier !== null) {
    return rate.list_price_multiplier === 0
      ? "$0 (count usage, no cost)"
      : `${rate.list_price_multiplier} × list price`;
  }
  const per = rate.meter === "sandbox.time" ? "per hour" : "per 1M tokens";
  return `${formatUsd(rate.unit_price_usd ?? 0)} ${per}`;
}

function AddRateForm({ onDone }: { onDone: () => void }) {
  const ui = useUiTranslations();
  const queryClient = useQueryClient();
  const [meter, setMeter] = useState<CostMeter>("model.tokens");
  const [provider, setProvider] = useState("");
  const [modelOrSize, setModelOrSize] = useState("");
  const [source, setSource] = useState("any");
  const [priceKind, setPriceKind] = useState<"fixed" | "multiple">("multiple");
  const [price, setPrice] = useState("");
  const [starts, setStarts] = useState("");
  const [reason, setReason] = useState("");
  const create = useMutation({
    mutationFn: (input: CreateCostRateRequest) => createCostRate(input, reason),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["costs"] });
      onDone();
    },
  });
  const isSandbox = meter === "sandbox.time";
  const amount = Number(price);
  const valid = price.trim() !== "" && Number.isFinite(amount) && amount >= 0;

  const submit = () => {
    const match: Record<string, string> = {};
    if (provider.trim()) match.provider = provider.trim();
    if (modelOrSize.trim()) match[isSandbox ? "size" : "model"] = modelOrSize.trim();
    if (!isSandbox && source !== "any") match.source = source;
    create.mutate({
      meter,
      match,
      ...(priceKind === "multiple" && !isSandbox
        ? { list_price_multiplier: amount }
        : { unit_price_usd: amount }),
      effective_from: starts ? new Date(`${starts}T00:00:00`).toISOString() : undefined,
    });
  };

  return (
    <div className="space-y-4 px-4 pb-6">
      <div className="space-y-1.5">
        <Label>{ui("What it prices")}</Label>
        <SegmentedControl
          label={ui("What it prices")}
          value={meter}
          onValueChange={(value) => setMeter(value as CostMeter)}
          options={[
            { value: "model.tokens", label: ui("Model tokens") },
            { value: "sandbox.time", label: ui("Sandbox time") },
          ]}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="rate-provider">{ui(isSandbox ? "Sandbox provider" : "Provider")}</Label>
        <Input
          id="rate-provider"
          value={provider}
          placeholder={isSandbox ? "daytona, e2b, modal" : "openai, anthropic, openrouter"}
          onChange={(e) => setProvider(e.target.value)}
        />
        <p className="text-xs text-muted-foreground">
          {ui("Leave empty to cover every provider.")}
        </p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="rate-detail">{ui(isSandbox ? "Size" : "Model")}</Label>
        <Input
          id="rate-detail"
          value={modelOrSize}
          placeholder={isSandbox ? ui("Template size, for example small") : "gpt-6.1-sol"}
          onChange={(e) => setModelOrSize(e.target.value)}
        />
      </div>
      {!isSandbox ? (
        <div className="space-y-1.5">
          <Label>{ui("Which keys")}</Label>
          <SegmentedControl
            label={ui("Which keys")}
            value={source}
            onValueChange={setSource}
            options={[
              { value: "any", label: ui("Any") },
              { value: "own_keys", label: ui("Own keys") },
              { value: "managed", label: ui("Built-in models") },
            ]}
          />
        </div>
      ) : null}
      <div className="space-y-1.5">
        <Label htmlFor="rate-price">{ui("Price")}</Label>
        {!isSandbox ? (
          <SegmentedControl
            label={ui("Price")}
            value={priceKind}
            onValueChange={(value) => setPriceKind(value as "fixed" | "multiple")}
            options={[
              { value: "multiple", label: ui("Multiple of list price") },
              { value: "fixed", label: ui("Fixed price") },
            ]}
          />
        ) : null}
        <Input
          id="rate-price"
          inputMode="decimal"
          value={price}
          placeholder={priceKind === "multiple" && !isSandbox ? "0.8" : "0.06"}
          onChange={(e) => setPrice(e.target.value)}
        />
        <p className="text-xs text-muted-foreground">
          {priceKind === "multiple" && !isSandbox
            ? ui(
                "0.8 is a 20% discount on the provider's or catalog price. 0 counts usage at no cost.",
              )
            : ui(isSandbox ? "US dollars per hour running." : "US dollars per million tokens.")}
        </p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="rate-starts">{ui("Starts")}</Label>
        <Input
          id="rate-starts"
          type="date"
          value={starts}
          onChange={(e) => setStarts(e.target.value)}
        />
        <p className="text-xs text-muted-foreground">{ui("Leave empty to start now.")}</p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="rate-reason">{ui("Why (shown in history)")}</Label>
        <Input id="rate-reason" value={reason} onChange={(e) => setReason(e.target.value)} />
      </div>
      {create.error ? (
        <p role="alert" className="text-sm text-destructive">
          {create.error.message}
        </p>
      ) : null}
      <div className="flex gap-2">
        <Button onClick={submit} disabled={!valid || create.isPending}>
          {ui("Add rate")}
        </Button>
        <Button variant="outline" onClick={onDone}>
          {ui("Cancel")}
        </Button>
      </div>
    </div>
  );
}

export function PriceList({ canManage }: { canManage: boolean }) {
  const ui = useUiTranslations();
  const queryClient = useQueryClient();
  const [adding, setAdding] = useState(false);
  const rates = useQuery({ queryKey: ["costs", "rates"], queryFn: listCostRates });
  const retire = useMutation({
    mutationFn: (rate: CostRate) => retireCostRate(rate.id),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ["costs"] }),
  });
  const [now] = useState(() => Date.now());

  return (
    <div className="space-y-6">
      <section className="border bg-card">
        <div className="flex items-center justify-between gap-3 border-b px-4 py-3">
          <div>
            <h2 className="text-sm font-semibold">{ui("Your rates")}</h2>
            <p className="text-xs text-muted-foreground">
              {ui(
                "Rates are never edited. A change is a new rate from a later date, so past costs keep the rate they used.",
              )}
            </p>
          </div>
          {canManage ? (
            <Button size="sm" onClick={() => setAdding(true)}>
              <Plus className="size-4" />
              {ui("Add rate")}
            </Button>
          ) : null}
        </div>
        {rates.isLoading ? (
          <Skeleton className="m-4 h-24" />
        ) : rates.error ? (
          <p role="alert" className="px-4 py-3 text-sm">
            {ui("Unable to load the price list.")}
          </p>
        ) : (rates.data ?? []).length === 0 ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">
            {ui(
              "No rates yet. Without one, costs come from the provider or the model catalog, and sandboxes are not priced.",
            )}
          </p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{ui("Applies to")}</TableHead>
                <TableHead>{ui("Rate")}</TableHead>
                <TableHead>{ui("From")}</TableHead>
                <TableHead>{ui("Why")}</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {(rates.data ?? []).map((rate) => {
                const scheduled = new Date(rate.effective_from).getTime() > now;
                return (
                  <TableRow
                    key={rate.id}
                    className={rate.retired ? "text-muted-foreground" : undefined}
                  >
                    <TableCell>{describeRateMatch(rate)}</TableCell>
                    <TableCell>{describeRatePrice(rate)}</TableCell>
                    <TableCell>
                      {DATE.format(new Date(rate.effective_from))}
                      {scheduled ? (
                        <Badge variant="outline" className="ml-2">
                          {ui("Scheduled")}
                        </Badge>
                      ) : null}
                    </TableCell>
                    <TableCell className="text-muted-foreground">{rate.reason ?? ""}</TableCell>
                    <TableCell className="text-right">
                      {canManage && !rate.retired ? (
                        <Button
                          size="sm"
                          variant="ghost"
                          disabled={retire.isPending}
                          onClick={() => retire.mutate(rate)}
                        >
                          {ui("Retire")}
                        </Button>
                      ) : null}
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </section>

      <section className="border bg-card">
        <div className="border-b px-4 py-3">
          <h2 className="text-sm font-semibold">{ui("Everything else")}</h2>
          <p className="text-xs text-muted-foreground">
            {ui("Where a cost comes from when no rate covers it, in this order.")}
          </p>
        </div>
        <ul className="divide-y">
          {(Object.keys(BASIS_DESCRIPTIONS) as (keyof typeof BASIS_DESCRIPTIONS)[]).map((basis) => (
            <li key={basis} className="flex items-center gap-4 px-4 py-2 text-sm">
              <BasisBadge basis={basis} className="w-40 justify-center" />
              <span>{ui(BASIS_DESCRIPTIONS[basis])}</span>
            </li>
          ))}
        </ul>
      </section>

      <Drawer open={adding} onOpenChange={setAdding}>
        <DrawerContent className="w-full overflow-y-auto sm:max-w-lg">
          <DrawerHeader className="pr-6">
            <DrawerTitle>{ui("Add rate")}</DrawerTitle>
            <DrawerDescription>
              {ui(
                "Your own internal rate. It changes what usage costs on these pages and in usage policies, not what any provider bills.",
              )}
            </DrawerDescription>
          </DrawerHeader>
          {adding ? <AddRateForm onDone={() => setAdding(false)} /> : null}
        </DrawerContent>
      </Drawer>
    </div>
  );
}
