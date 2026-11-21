export const messages: Readonly<Record<string, string>> = {
  "Compaction quality monitoring": "Qualitätsüberwachung der Komprimierung",
  "Automatically check compaction in real sessions. Operational checks use no judge model; sampled continuity checks use the organization model selected below.":
    "Prüfen Sie die Komprimierung automatisch in echten Sitzungen. Betriebsprüfungen verwenden kein Bewertungsmodell; stichprobenartige Kontinuitätsprüfungen nutzen das unten gewählte Organisationsmodell.",
  "Findings stay in this organization. Sampled excerpts are processed by its selected provider. Monitoring is read-only, and findings are retained for 30 days.":
    "Ergebnisse bleiben in dieser Organisation. Stichprobenauszüge werden von ihrem ausgewählten Anbieter verarbeitet. Die Überwachung ist schreibgeschützt; Ergebnisse werden 30 Tage aufbewahrt.",
  "Enable automatic monitoring": "Automatische Überwachung aktivieren",
  "Monitoring model": "Überwachungsmodell",
  "Mechanical checks only": "Nur mechanische Prüfungen",
  "Daily monitoring budget (USD)": "Tägliches Überwachungsbudget (USD)",
  "Continuity sample (%)": "Kontinuitätsstichprobe (%)",
  "Excluded session tags": "Ausgeschlossene Sitzungs-Tags",
  "A zero budget enables mechanical checks only. The daily UTC cap includes conservative retry reservations; uncertain calls keep their allowance for that day. Disabling monitoring stops new assessments, including queued work.":
    "Ein Nullbudget aktiviert nur mechanische Prüfungen. Das tägliche UTC-Limit enthält vorsichtige Reservierungen für Wiederholungen; unklare Aufrufe behalten ihre Reservierung für diesen Tag. Das Deaktivieren stoppt neue Bewertungen, einschließlich wartender Aufträge.",
  "Monitoring coverage · 7 days": "Überwachungsabdeckung · 7 Tage",
  "Could not load monitoring coverage.": "Überwachungsabdeckung konnte nicht geladen werden.",
  "Today: {reserved} reserved allowance · {reported} provider-reported cost":
    "Heute: {reserved} reserviert · {reported} vom Anbieter gemeldete Kosten",
  "No compaction assessments yet.": "Noch keine Komprimierungsbewertungen.",
  "Continuity checks": "Kontinuitätsprüfungen",
  "Operational checks": "Betriebsprüfungen",
  "Passed {passed} · Failed {failed} · Insufficient evidence {unknown} · Pending {pending}":
    "Bestanden {passed} · Fehlgeschlagen {failed} · Unzureichende Belege {unknown} · Ausstehend {pending}",
  "Recent findings": "Neueste Ergebnisse",
  "An operational pass confirms the reported execution step. Continuity verdicts apply to sampled evidence and do not prove complete memory preservation or causality.":
    "Eine bestandene Betriebsprüfung bestätigt den gemeldeten Ausführungsschritt. Kontinuitätsurteile gelten für Stichprobenbelege und beweisen weder vollständige Erinnerung noch Kausalität.",
  "Could not load monitoring findings.": "Überwachungsergebnisse konnten nicht geladen werden.",
  "Insufficient evidence": "Unzureichende Belege",
  "Select a monitoring model before enabling paid checks.":
    "Wählen Sie ein Überwachungsmodell, bevor Sie kostenpflichtige Prüfungen aktivieren.",
  "Compaction monitoring": "Komprimierungsüberwachung",
  "System-managed checks configured in organization settings.":
    "Systemverwaltete Prüfungen werden in den Organisationseinstellungen konfiguriert.",
  "Agree with assessment": "Bewertung zustimmen",
  "Disagree with assessment": "Bewertung widersprechen",
  "Review recorded": "Rückmeldung gespeichert",
  "Could not save assessment review.": "Rückmeldung zur Bewertung konnte nicht gespeichert werden.",
};
