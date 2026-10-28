"use client";

import { useLocale } from "@/providers/locale-provider";
import { interfaceLocales, isSupportedLocale } from "@/lib/locale-registry";
import { Label } from "@/components/ui/label";

/** UI language is a device preference, independent of the runtime virtual-user locale. */
export function LanguageSettings() {
  const { preference, setPreference, t } = useLocale();
  return (
    <section className="space-y-2">
      <Label htmlFor="ui-language">{t("interface_language")}</Label>
      <select
        id="ui-language"
        value={preference}
        className="block h-9 rounded-md border bg-background px-3 text-sm"
        onChange={(event) => {
          const value = event.target.value;
          if (value === "system" || isSupportedLocale(value)) setPreference(value);
        }}
      >
        <option value="system">{t("interface_language_system")}</option>
        {Object.entries(interfaceLocales).map(([value, label]) => (
          <option key={value} value={value}>
            {label}
          </option>
        ))}
      </select>
      <p className="text-sm text-muted-foreground">{t("interface_language_help")}</p>
    </section>
  );
}
