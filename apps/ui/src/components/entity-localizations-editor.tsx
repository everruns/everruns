"use client";

import { useId, useState } from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Button } from "@/components/ui/button";
import { useLocale } from "@/providers/locale-provider";
import type { EntityLocalizations } from "@/lib/entity-localization";

export function EntityLocalizationsEditor({
  value,
  onChange,
  readOnly = false,
}: {
  value: EntityLocalizations;
  onChange: (value: EntityLocalizations) => void;
  readOnly?: boolean;
}) {
  const { t } = useLocale();
  const id = useId();
  const [tag, setTag] = useState("");
  const [error, setError] = useState("");
  function add() {
    try {
      const locale = Intl.getCanonicalLocales(tag.trim().replace(/_/g, "-"))[0]?.toLowerCase();
      if (!locale || value[locale] || Object.keys(value).length >= 64) throw new Error();
      onChange({ ...value, [locale]: {} });
      setTag("");
      setError("");
    } catch {
      setError(t("localization_invalid_locale"));
    }
  }
  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("localization_help")}</p>
      {Object.entries(value).map(([locale, entry]) => (
        <fieldset key={locale} disabled={readOnly} className="space-y-3 rounded-lg border p-4">
          <legend className="px-1 font-medium">{locale}</legend>
          <div className="space-y-2">
            <Label htmlFor={`${id}-${locale}-name`}>{t("localization_display_name")}</Label>
            <Input
              id={`${id}-${locale}-name`}
              maxLength={255}
              value={entry.display_name ?? ""}
              onChange={(event) =>
                onChange({
                  ...value,
                  [locale]: { ...entry, display_name: event.target.value || undefined },
                })
              }
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor={`${id}-${locale}-description`}>{t("localization_description")}</Label>
            <Textarea
              id={`${id}-${locale}-description`}
              maxLength={2000}
              value={entry.description ?? ""}
              onChange={(event) =>
                onChange({
                  ...value,
                  [locale]: { ...entry, description: event.target.value || undefined },
                })
              }
            />
          </div>
          {!readOnly && (
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                const next = { ...value };
                delete next[locale];
                onChange(next);
              }}
            >
              {t("localization_remove")}
            </Button>
          )}
        </fieldset>
      ))}
      {!readOnly && (
        <div className="space-y-2">
          <Label htmlFor={`${id}-locale`}>{t("localization_locale")}</Label>
          <div className="flex gap-2">
            <Input
              id={`${id}-locale`}
              value={tag}
              placeholder="uk, de, fr-CA"
              aria-invalid={!!error}
              aria-describedby={error ? `${id}-error` : undefined}
              onChange={(event) => {
                setTag(event.target.value);
                setError("");
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  add();
                }
              }}
            />
            <Button type="button" variant="outline" onClick={add}>
              {t("localization_add")}
            </Button>
          </div>
          {error && (
            <p id={`${id}-error`} role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
