// German product copy for background limits (Settings → Limits, schedule limits).
export const messages: Readonly<Record<string, string>> = {
  Limits: "Limits",
  "Limit what schedules and other unattended work may run and spend":
    "Begrenzen, was Zeitpläne und andere unbeaufsichtigte Arbeit ausführen und ausgeben dürfen",
  "Unable to load limits.": "Limits konnten nicht geladen werden.",
  "Unattended work": "Unbeaufsichtigte Arbeit",
  "Limits for work no person started: schedules agents create, trigger runs and automatic retries. A person can still extend a schedule from its session.":
    "Limits für Arbeit, die keine Person gestartet hat: von Agents erstellte Zeitpläne, Trigger-Läufe und automatische Wiederholungen. Eine Person kann einen Zeitplan weiterhin in seiner Sitzung verlängern.",
  "Longest an agent's schedule may run": "Längste Laufzeit eines Agent-Zeitplans",
  "Up to 7 days. The agent must set an end, and it cannot ask for more than this.":
    "Bis zu 7 Tage. Der Agent muss ein Ende festlegen und kann nicht mehr anfordern.",
  "Most runs per agent schedule": "Höchstzahl der Läufe pro Agent-Zeitplan",
  "Up to 96 runs. The schedule stops at this count or at its end time, whichever is first.":
    "Bis zu 96 Läufe. Der Zeitplan stoppt bei dieser Anzahl oder zu seiner Endzeit, je nachdem, was zuerst eintritt.",
  "Default budget per schedule": "Standardbudget pro Zeitplan",
  "What a schedule may spend in total when the agent does not ask for a budget.":
    "Was ein Zeitplan insgesamt ausgeben darf, wenn der Agent kein Budget anfordert.",
  "Largest budget an agent may give a schedule":
    "Höchstes Budget, das ein Agent einem Zeitplan geben darf",
  "The most an agent may ask for. A person can raise a single schedule past it.":
    "Das Maximum, das ein Agent anfordern darf. Eine Person kann einen einzelnen Zeitplan darüber hinaus anheben.",
  "Daily cap on unattended work (schedules, triggers, retries)":
    "Tageslimit für unbeaufsichtigte Arbeit (Zeitpläne, Trigger, Wiederholungen)",
  "What all unattended work in this organization may spend per day, together.":
    "Was die gesamte unbeaufsichtigte Arbeit dieser Organisation pro Tag zusammen ausgeben darf.",
  "No cap": "Kein Limit",
  "Only organization owners and admins can change these limits.":
    "Nur Eigentümer und Admins der Organisation können diese Limits ändern.",
  "Could not save limits.": "Limits konnten nicht gespeichert werden.",
  runs: "Läufe",
  "Schedule lifetime must be a whole number of days from 1 to 7.":
    "Die Laufzeit eines Zeitplans muss eine ganze Zahl von Tagen zwischen 1 und 7 sein.",
  "Runs per schedule must be a whole number from 1 to 96.":
    "Die Läufe pro Zeitplan müssen eine ganze Zahl zwischen 1 und 96 sein.",
  "Default budget per schedule must be more than $0.":
    "Das Standardbudget pro Zeitplan muss über 0 $ liegen.",
  "Largest schedule budget must be more than $0.":
    "Das höchste Zeitplanbudget muss über 0 $ liegen.",
  "Default budget per schedule cannot be more than the largest budget.":
    "Das Standardbudget pro Zeitplan darf das höchste Budget nicht übersteigen.",
  "Daily cap must be more than $0, or turn on No cap.":
    "Das Tageslimit muss über 0 $ liegen, oder schalten Sie Kein Limit ein.",
  "Unattended work (daily)": "Unbeaufsichtigte Arbeit (täglich)",
  "Usage limit reached": "Nutzungslimit erreicht",
  "Stopped: repeating": "Gestoppt: wiederholt sich",
  " · Run {value0} of {value1}": " · Lauf {value0} von {value1}",
  " · Ends {value0}": " · Endet {value0}",
  " · {value0} of {value1}": " · {value0} von {value1}",
  "Stops when: {value0}": "Stoppt, wenn: {value0}",
  Extend: "Verlängern",
  "Re-enable": "Wieder aktivieren",
  "Could not re-enable the schedule.": "Der Zeitplan konnte nicht wieder aktiviert werden.",
  "Extend schedule": "Zeitplan verlängern",
  "Give this schedule more time, runs or budget. You can go past the limits agents are held to.":
    "Geben Sie diesem Zeitplan mehr Zeit, Läufe oder Budget. Sie können über die Limits hinausgehen, die für Agents gelten.",
  "Ends at": "Endet am",
  "Most runs": "Höchstzahl der Läufe",
  "Runs so far: {value0}": "Bisherige Läufe: {value0}",
  "Budget (USD)": "Budget (USD)",
  "Could not extend the schedule.": "Der Zeitplan konnte nicht verlängert werden.",
  "The end time must be in the future.": "Die Endzeit muss in der Zukunft liegen.",
  "Runs must be a whole number above 0.": "Die Läufe müssen eine ganze Zahl über 0 sein.",
  "The budget must be more than $0.": "Das Budget muss über 0 $ liegen.",
  "Change at least one value to extend.": "Ändern Sie mindestens einen Wert, um zu verlängern.",
};
