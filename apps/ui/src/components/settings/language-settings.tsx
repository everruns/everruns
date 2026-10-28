"use client";

import { useLocale } from "@/providers/locale-provider";
import { interfaceLocales } from "@/lib/locale-registry";
import { Label } from "@/components/ui/label";
import { z } from "zod";
import { usePageSettingsDraft } from "@/components/chat-mode/use-page-settings-draft";

const schema = z.strictObject({ language: z.union([z.literal("system"), z.enum(interfaceLocales)]) });

/** UI language is a device preference, independent of the runtime virtual-user locale. */
export function LanguageSettings() {
  const { preference, setPreference, t } = useLocale();
  const draft = usePageSettingsDraft({
    schema,
    values: { language: preference },
    entityType: "device_preferences",
    formId: "interface_language",
    label: t("interface_language"),
    writable: true,
    save: async (next) => {
      setPreference(next.language);
    },
  });
  return (
    <section className="space-y-2">
      <Label htmlFor="ui-language">{t("interface_language")}</Label>
      <select
        id="ui-language"
        value={draft.values.language}
        className="block h-9 rounded-md border bg-background px-3 text-sm"
        disabled={draft.pending}
        onChange={(event) => {
          const language = schema.shape.language.parse(event.target.value);
          if (draft.adopted) draft.setValues({ language });
          else setPreference(language);
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
      {draft.controls}
    </section>
  );
}
