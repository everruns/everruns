"use client";

import Form from "@rjsf/core";
import validator from "@rjsf/validator-ajv8";
import type {
  FieldTemplateProps,
  RJSFSchema,
  RJSFValidationError,
  UiSchema,
  WidgetProps,
} from "@rjsf/utils";
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
import { localizedConfigDescription, localizedConfigSchema } from "@/lib/capability-localization";
import { formatMessage, type MessageKey, type SupportedLocale } from "@/lib/i18n";
import { useLocale } from "@/providers/locale-provider";
import { MemoryConfigEditor } from "./memory-config-editor";

// Capability IDs with purpose-built editors. Mirrors the constants exported
// from crates/core/src/capabilities/*.
const MEMORY_CAPABILITY_ID = "memory";

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
  if (capability.id === MEMORY_CAPABILITY_ID) {
    return <MemoryConfigEditor config={config} onChange={onChange} disabled={disabled} />;
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
  const { locale } = useLocale();
  const schema = localizedConfigSchema(capability, locale);
  if (!schema) {
    return null;
  }
  const configDescription = localizedConfigDescription(capability, locale);

  const uiSchema: UiSchema = {
    ...(capability.config_ui_schema as UiSchema | undefined),
    "ui:submitButtonOptions": { norender: true },
  };

  return (
    <div className="pt-2">
      {configDescription && (
        <p className="pb-2 text-xs text-muted-foreground">{configDescription}</p>
      )}
      <Form
        schema={schema as RJSFSchema}
        uiSchema={uiSchema}
        formData={config}
        validator={validator}
        disabled={disabled}
        templates={{ FieldTemplate }}
        widgets={CAPABILITY_CONFIG_WIDGETS}
        showErrorList={false}
        noHtml5Validate
        transformErrors={(errors) => translateValidationErrors(locale, errors)}
        onChange={({ formData }) => onChange(cleanConfig(formData))}
      />
    </div>
  );
}

// Maps AJV error keywords to catalog message keys; unknown keywords keep the
// validator's original message so information is never lost.
const VALIDATION_MESSAGE_KEYS: Record<string, MessageKey> = {
  required: "validation_required",
  pattern: "validation_pattern",
  enum: "validation_enum",
  const: "validation_enum",
  oneOf: "validation_enum",
  type: "validation_type",
  minimum: "validation_minimum",
  exclusiveMinimum: "validation_minimum",
  maximum: "validation_maximum",
  exclusiveMaximum: "validation_maximum",
  minLength: "validation_min_length",
  maxLength: "validation_max_length",
  minItems: "validation_min_items",
  maxItems: "validation_max_items",
};

function validationMessageValue(error: RJSFValidationError): string | number {
  const params = (error.params ?? {}) as Record<string, unknown>;
  const limit = params.limit ?? params.type ?? params.allowedValues;
  if (typeof limit === "number" || typeof limit === "string") return limit;
  if (Array.isArray(limit)) return limit.join(", ");
  return "";
}

export function translateValidationErrors(
  locale: SupportedLocale,
  errors: RJSFValidationError[],
): RJSFValidationError[] {
  return errors.map((error) => {
    const key = error.name ? VALIDATION_MESSAGE_KEYS[error.name] : undefined;
    if (!key) return error;
    const message = formatMessage(locale, key, { value: validationMessageValue(error) });
    return { ...error, message, stack: message };
  });
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
