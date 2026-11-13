"use client";

import { useState } from "react";
import { useOrg } from "@/providers/org-provider";
import { usePageTitle } from "@/hooks";
import { useUiTranslations } from "@/providers/locale-provider";
import { ModelPicker } from "@/components/models/model-picker";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { PageBreadcrumb, PageContainer, PageMasthead } from "@/components/layout/page-layout";
import { api } from "@/lib/api/client";

const example = JSON.stringify(
  {
    total: { type: "number", instructions: "The invoice total", unit: "USD" },
    due_date: { type: "date", instructions: "The payment due date" },
  },
  null,
  2,
);

export default function ExtractionsPage() {
  const { currentOrg } = useOrg();
  return <ExtractionForm key={currentOrg?.public_id ?? "none"} />;
}

function ExtractionForm() {
  const ui = useUiTranslations();
  usePageTitle(ui("Extractions"));
  const [model, setModel] = useState("");
  const [state, setState] = useState("");
  const [fields, setFields] = useState(example);
  const [result, setResult] = useState<unknown>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);

  async function run() {
    setError(false);
    setResult(undefined);
    setBusy(true);
    try {
      const parsed: unknown = JSON.parse(fields);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error();
      const response = await api.post<unknown>("/v1/extractions", {
        model_id: model,
        input: { state, fields: parsed },
      });
      setResult(response.data);
    } catch {
      setError(true);
    } finally {
      setBusy(false);
    }
  }

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Extractions") }]} />
      <PageMasthead
        title={ui("Extractions")}
        description={ui("Extract typed values and evidence using your provider account.")}
      />
      <div className="max-w-3xl space-y-4 p-4">
        <p className="text-sm text-muted-foreground">
          {ui(
            "Requires model management permission and an organization-owned provider. Provider charges may apply.",
          )}
        </p>
        <Label htmlFor="extraction-model">{ui("Model")}</Label>
        <ModelPicker
          id="extraction-model"
          service="extractions"
          value={model}
          onChange={setModel}
          disabled={busy}
        />
        <Label htmlFor="extraction-state">{ui("Source text")}</Label>
        <Textarea
          id="extraction-state"
          value={state}
          onChange={(e) => setState(e.target.value)}
          rows={8}
          disabled={busy}
        />
        <Label htmlFor="extraction-fields">{ui("Fields (JSON)")}</Label>
        <Textarea
          id="extraction-fields"
          value={fields}
          onChange={(e) => setFields(e.target.value)}
          rows={10}
          className="font-mono"
          disabled={busy}
        />
        <Button onClick={run} disabled={busy || !model || !state.trim()}>
          {ui(busy ? "Extracting…" : "Extract")}
        </Button>
        {error && (
          <p role="alert" className="text-destructive">
            {ui(
              "Extraction failed. Check the field schema, provider configuration and permissions.",
            )}
          </p>
        )}
        {result !== undefined && (
          <section aria-label={ui("Extraction result")} className="space-y-2">
            <h2 className="font-medium">{ui("Extraction result")}</h2>
            <p className="text-sm text-muted-foreground">
              {ui("Missing confidence means unknown. Evidence offsets count Unicode code points.")}
            </p>
            <pre className="overflow-auto whitespace-pre-wrap rounded border p-4 text-sm">
              {JSON.stringify(result, null, 2)}
            </pre>
          </section>
        )}
      </div>
    </PageContainer>
  );
}
