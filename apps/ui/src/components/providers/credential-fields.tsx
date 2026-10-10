"use client";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { CredentialFormField, CredentialFormSchema } from "@/lib/api/types";

// Renders a driver's declared credential schema as discrete typed inputs.
//
// Fields without a `group` are always part of the credential (e.g. Bedrock's
// AWS keys). Fields sharing a `group` label are one mutually-exclusive method
// (e.g. MAI's "API key" vs "Microsoft Entra ID OAuth"); they render under the
// group heading with an "or" separator, and are not natively required so the
// operator can fill just one method. The backend validates the chosen method.

function inputType(fieldType: CredentialFormField["field_type"]): string {
  return fieldType === "password" ? "password" : fieldType === "url" ? "url" : "text";
}

function CredentialField({
  field,
  value,
  onChange,
  idPrefix,
  nativeRequired,
}: {
  field: CredentialFormField;
  value: string;
  onChange: (name: string, value: string) => void;
  idPrefix: string;
  nativeRequired: boolean;
}) {
  const id = `${idPrefix}-${field.name}`;
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>
        {field.label}
        {/* Grouped required fields are required once the user picks that method,
            so only mark genuinely optional fields. */}
        {field.required ? null : (
          <span className="ml-1 text-xs text-muted-foreground">(optional)</span>
        )}
      </Label>
      <Input
        id={id}
        type={inputType(field.field_type)}
        value={value}
        onChange={(e: React.ChangeEvent<HTMLInputElement>) => onChange(field.name, e.target.value)}
        placeholder={field.placeholder}
        required={nativeRequired}
      />
      {field.help_text ? <p className="text-xs text-muted-foreground">{field.help_text}</p> : null}
    </div>
  );
}

export function CredentialFields({
  schema,
  values,
  onChange,
  idPrefix,
  allowEmptySubmit = false,
}: {
  schema: CredentialFormSchema;
  values: Record<string, string>;
  onChange: (name: string, value: string) => void;
  idPrefix: string;
  /**
   * OAuth-capable add flows can intentionally create a provider before any
   * credential exists, then connect the credential in a follow-up browser flow.
   */
  allowEmptySubmit?: boolean;
}) {
  const ungrouped = schema.fields.filter((f) => !f.group);
  // Preserve declaration order of group labels.
  const groupLabels: string[] = [];
  for (const field of schema.fields) {
    if (field.group && !groupLabels.includes(field.group)) groupLabels.push(field.group);
  }
  // Label each credential method when there is more than one way to authenticate
  // (multiple groups, or groups alongside always-present ungrouped fields). A
  // schema with a single group and nothing else needs no heading.
  const showGroupHeadings = groupLabels.length > 1 || ungrouped.length > 0;

  return (
    <div className="space-y-4">
      {ungrouped.map((field) => (
        <CredentialField
          key={field.name}
          field={field}
          value={values[field.name] ?? ""}
          onChange={onChange}
          idPrefix={idPrefix}
          nativeRequired={field.required && !field.group && !allowEmptySubmit}
        />
      ))}
      {groupLabels.map((label, index) => (
        <div key={label} className="space-y-4">
          {showGroupHeadings && (
            <div className="flex items-center gap-3 py-1">
              <div className="h-px flex-1 bg-border" />
              <span className="text-xs text-muted-foreground">
                {index === 0 && ungrouped.length === 0 ? label : `or ${label}`}
              </span>
              <div className="h-px flex-1 bg-border" />
            </div>
          )}
          {schema.fields
            .filter((f) => f.group === label)
            .map((field) => (
              <CredentialField
                key={field.name}
                field={field}
                value={values[field.name] ?? ""}
                onChange={onChange}
                idPrefix={idPrefix}
                nativeRequired={field.required && !field.group && !allowEmptySubmit}
              />
            ))}
        </div>
      ))}
    </div>
  );
}

/** Seed form values from a schema's declared `default_value`s.
 *
 * Only ungrouped fields are pre-filled: the backend treats any non-empty
 * grouped field as selecting that credential method, so pre-filling a grouped
 * field's default (e.g. an OAuth scope/authority) would wrongly force that
 * method's validation even when the user picks a different one. Grouped
 * defaults are surfaced as input placeholders instead, and the driver applies
 * them when the field is left blank. */
export function defaultCredentialValues(
  schema: CredentialFormSchema | undefined,
): Record<string, string> {
  const values: Record<string, string> = {};
  for (const field of schema?.fields ?? []) {
    if (field.default_value && !field.group) values[field.name] = field.default_value;
  }
  return values;
}

/** Collect non-empty credential values to submit. */
export function nonEmptyCredentials(values: Record<string, string>): Record<string, string> {
  return Object.fromEntries(Object.entries(values).filter(([, v]) => v.trim() !== ""));
}
