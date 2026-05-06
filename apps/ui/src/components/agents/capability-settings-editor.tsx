"use client";

import Form from "@rjsf/core";
import validator from "@rjsf/validator-ajv8";
import type { FieldTemplateProps, RJSFSchema, UiSchema, WidgetProps } from "@rjsf/utils";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import type { Capability } from "@/lib/api/types";
import { WorkspaceVolumesConfigEditor } from "./workspace-volumes-config-editor";

// Capability IDs with purpose-built editors. Mirrors the constants exported
// from crates/core/src/capabilities/*.
const WORKSPACE_VOLUMES_CAPABILITY_ID = "workspace_volumes";

interface CapabilitySettingsEditorProps {
  /** Full capability metadata, including optional config schema */
  capability: Capability;
  /** Current config value */
  config: Record<string, unknown>;
  /** Callback when config changes */
  onChange: (config: Record<string, unknown>) => void;
  /** Whether editing is disabled */
  disabled?: boolean;
}

export function CapabilitySettingsEditor({
  capability,
  config,
  onChange,
  disabled,
}: CapabilitySettingsEditorProps) {
  if (capability.id === WORKSPACE_VOLUMES_CAPABILITY_ID) {
    return <WorkspaceVolumesConfigEditor config={config} onChange={onChange} disabled={disabled} />;
  }
  return (
    <SchemaCapabilityEditor
      capability={capability}
      config={config}
      onChange={onChange}
      disabled={disabled}
    />
  );
}

export function hasCapabilitySettings(capability: Capability): boolean {
  return Boolean(capability.config_schema);
}

interface SchemaCapabilityEditorProps {
  capability: Capability;
  config: Record<string, unknown>;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}

function SchemaCapabilityEditor({
  capability,
  config,
  onChange,
  disabled,
}: SchemaCapabilityEditorProps) {
  if (!capability.config_schema) {
    return null;
  }

  const uiSchema: UiSchema = {
    ...(capability.config_ui_schema as UiSchema | undefined),
    "ui:submitButtonOptions": { norender: true },
  };

  return (
    <div className="pt-2">
      <Form
        schema={capability.config_schema as RJSFSchema}
        uiSchema={uiSchema}
        formData={config}
        validator={validator}
        disabled={disabled}
        templates={{ FieldTemplate }}
        widgets={CAPABILITY_CONFIG_WIDGETS}
        showErrorList={false}
        noHtml5Validate
        onChange={({ formData }) => onChange(cleanConfig(formData))}
      />
    </div>
  );
}

function cleanConfig(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }

  return Object.fromEntries(
    Object.entries(value).filter(([, fieldValue]) => fieldValue !== undefined),
  );
}

const CAPABILITY_CONFIG_WIDGETS = {
  CheckboxWidget,
  EmailWidget: TextWidget,
  PasswordWidget: TextWidget,
  SelectWidget,
  TextareaWidget,
  TextWidget,
  URLWidget: TextWidget,
  UpDownWidget: TextWidget,
};

function FieldTemplate({
  id,
  label,
  required,
  description,
  help,
  errors,
  children,
  hidden,
}: FieldTemplateProps) {
  if (hidden) {
    return <>{children}</>;
  }

  return (
    <div className="space-y-1.5">
      {label && (
        <Label htmlFor={id} className="text-xs font-normal text-muted-foreground">
          {label}
          {required && <span className="ml-0.5 text-destructive">*</span>}
        </Label>
      )}
      {children}
      {description}
      {errors}
      {help}
    </div>
  );
}

function TextWidget({
  id,
  value,
  disabled,
  readonly,
  onChange,
  onBlur,
  onFocus,
  placeholder,
  options,
  schema,
}: WidgetProps) {
  const inputType =
    typeof options.inputType === "string"
      ? options.inputType
      : schema.type === "number" || schema.type === "integer"
        ? "number"
        : "text";

  return (
    <Input
      id={id}
      type={inputType}
      value={value ?? ""}
      placeholder={placeholder}
      disabled={disabled || readonly}
      className="h-8 text-sm"
      onBlur={(event) => onBlur(id, event.target.value)}
      onFocus={(event) => onFocus(id, event.target.value)}
      onChange={(event) => {
        const nextValue = event.target.value;
        if (inputType === "number") {
          const parsedValue = Number(nextValue);
          onChange(nextValue === "" || !Number.isFinite(parsedValue) ? undefined : parsedValue);
        } else {
          onChange(nextValue);
        }
      }}
    />
  );
}

function TextareaWidget({
  id,
  value,
  disabled,
  readonly,
  onChange,
  onBlur,
  onFocus,
  placeholder,
}: WidgetProps) {
  return (
    <Textarea
      id={id}
      value={value ?? ""}
      placeholder={placeholder}
      disabled={disabled || readonly}
      className="min-h-20 text-sm"
      onBlur={(event) => onBlur(id, event.target.value)}
      onFocus={(event) => onFocus(id, event.target.value)}
      onChange={(event) => onChange(event.target.value)}
    />
  );
}

function SelectWidget({ id, value, disabled, readonly, onChange, options }: WidgetProps) {
  const enumOptions = options.enumOptions ?? [];
  const selectedValue = value === undefined || value === null ? undefined : String(value);
  const selectedOption = enumOptions.find((option) => String(option.value) === selectedValue);

  return (
    <Select
      value={selectedValue}
      onValueChange={(nextValue) => {
        const option = enumOptions.find((item) => String(item.value) === nextValue);
        onChange(option?.value ?? nextValue);
      }}
      disabled={disabled || readonly}
    >
      <SelectTrigger id={id} className="w-full">
        <SelectValue>{selectedOption?.label}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        {enumOptions.map((option) => (
          <SelectItem key={String(option.value)} value={String(option.value)}>
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function CheckboxWidget({ id, value, disabled, readonly, onChange }: WidgetProps) {
  return (
    <Checkbox
      id={id}
      checked={Boolean(value)}
      disabled={disabled || readonly}
      onCheckedChange={onChange}
    />
  );
}
