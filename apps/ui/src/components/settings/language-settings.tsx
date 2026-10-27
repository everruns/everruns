"use client";

import { useLocale } from "@/providers/locale-provider";
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
        onChange={(event) => setPreference(event.target.value as "system" | "en" | "uk")}
      >
        <option value="system">{t("interface_language_system")}</option>
        <option value="en">English</option>
        <option value="uk">Українська</option>
      </select>
      <p className="text-sm text-muted-foreground">{t("interface_language_help")}</p>
    </section>
  );
}
