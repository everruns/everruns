"use client";
import { useState } from "react";
import { Combobox } from "@/components/ui/combobox";
import { ResourceMultiSelect } from "@/components/ui/resource-multi-select";
import { TagInput } from "@/components/ui/tag-input";
import { Slider } from "@/components/ui/slider";
import { Field } from "@/components/ui/field";
import { Pagination } from "@/components/ui/pagination";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";

const resources = Array.from({ length: 30 }, (_, i) => ({
  value: `resource-${i}`,
  label: `Resource ${i + 1}`,
  description: "Example organization resource",
  disabled: i === 4,
}));
export function SharedPatterns() {
  const [value, setValue] = useState("");
  const [selected, setSelected] = useState(["resource-2", "missing-resource"]);
  const [tags, setTags] = useState("production, support");
  const [page, setPage] = useState(0);
  const [state, setState] = useState("ready");
  const [confirm, setConfirm] = useState(false);
  return (
    <section id="shared-patterns" className="space-y-4 border bg-card p-4">
      <h2 className="text-lg font-semibold">Shared product patterns</h2>
      <p className="text-sm text-muted-foreground">
        Production controls used by resource pickers, observer forms, Settings and lists. Search,
        select with the keyboard, and close with Escape.
      </p>
      <SegmentedControl
        label="Example state"
        value={state}
        onValueChange={setState}
        options={["ready", "loading", "error", "empty"].map((value) => ({ value, label: value }))}
      />
      <div className="grid gap-6 lg:grid-cols-2">
        <div className="space-y-4">
          <Field
            id="pattern-resource"
            label="Resource"
            description="Search names or identifiers; resource 5 is disabled."
          >
            <Combobox
              id="pattern-resource"
              aria-describedby="pattern-resource-hint"
              ariaLabel="Resource"
              value={value}
              onValueChange={setValue}
              options={state === "empty" ? [] : resources}
              loading={state === "loading"}
              error={state === "error" ? "Could not load resources. Try again." : undefined}
            />
          </Field>
          <ResourceMultiSelect
            label="Bound resources"
            value={selected}
            onChange={setSelected}
            options={resources}
          />
        </div>
        <div className="space-y-4">
          <Field id="pattern-tags" label="Tags">
            <TagInput id="pattern-tags" value={tags} onChange={setTags} />
          </Field>
          <Field id="pattern-sampling" label="Sampling rate">
            <Slider id="pattern-sampling" min={0} max={1} step={0.05} defaultValue={0.1} />
          </Field>
          <Button loading>Saving</Button>
          <Button variant="outline" onClick={() => setConfirm(true)}>
            Preview confirmation
          </Button>
          <Pagination page={page} totalPages={4} onPageChange={setPage} summary="30 resources" />
        </div>
      </div>
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title="Archive resource"
        description="This preview does not change product data."
        confirmLabel="Archive"
        onConfirm={async () => {}}
      />
    </section>
  );
}
