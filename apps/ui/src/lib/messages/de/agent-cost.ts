export const messages: Readonly<Record<string, string>> = {
  "/ task": "/ Aufgabe",
  "Advanced assumptions": "Erweiterte Annahmen",
  "Agent model": "Agentenmodell",
  "Allow for variation": "Schwankungen einplanen",
  "Answer a question or write a short reply":
    "Eine Frage beantworten oder eine kurze Antwort schreiben",
  "Calculating your estimate…": "Schätzung wird berechnet…",
  "Check the numbers under Advanced assumptions, or choose a task to reset them.":
    "Prüfen Sie die Zahlen unter Erweiterte Annahmen oder wählen Sie eine Aufgabe, um sie zurückzusetzen.",
  "Choose a model for this agent before estimating its cost.":
    "Wählen Sie ein Modell für diesen Agenten, bevor Sie die Kosten schätzen.",
  "Choose the kind of work and how often you’ll use this agent.":
    "Wählen Sie die Art der Arbeit und wie oft Sie diesen Agenten nutzen werden.",
  "Cost breakdown": "Kostenaufschlüsselung",
  "Deep research": "Gründliche Recherche",
  "Enter a whole number from 0 to 1,000,000.": "Geben Sie eine ganze Zahl von 0 bis 1.000.000 ein.",
  "Enter a whole number from {min} to {max}.": "Geben Sie eine ganze Zahl von {min} bis {max} ein.",
  "Estimate cost": "Kosten schätzen",
  "Estimated AI cost · USD": "Geschätzte KI-Kosten · USD",
  "Explore a topic and compare several sources":
    "Ein Thema untersuchen und mehrere Quellen vergleichen",
  "Find information and put together an answer":
    "Informationen suchen und eine Antwort zusammenstellen",
  "For simpler or more involved tasks": "Für einfachere oder aufwendigere Aufgaben",
  "Harness model": "Harness-Modell",
  "Include follow-up calls after tools.": "Folgeaufrufe nach Werkzeugen einbeziehen.",
  "Include reasoning and tool-call arguments.":
    "Denkschritte und Argumente der Werkzeugaufrufe einbeziehen.",
  "Input · {count} tokens": "Eingabe · {count} Token",
  "Larger tasks may need a different model.":
    "Größere Aufgaben benötigen möglicherweise ein anderes Modell.",
  "Long-context pricing applies to one or more calls.":
    "Für mindestens einen Aufruf gelten Preise für lange Kontexte.",
  "Model calls per task": "Modellaufrufe pro Aufgabe",
  "Monthly cost": "Monatliche Kosten",
  "Monthly cost assumes 30 days of use. Each task starts a new conversation.":
    "Die monatlichen Kosten gehen von 30 Nutzungstagen aus. Jede Aufgabe beginnt ein neues Gespräch.",
  "Multi-step task": "Mehrstufige Aufgabe",
  "New tool results added before the next call.":
    "Neue Werkzeugergebnisse vor dem nächsten Aufruf.",
  "One task is one request, such as “research this company.”":
    "Eine Aufgabe ist eine Anfrage, etwa „Recherchiere zu diesem Unternehmen“.",
  "Output is capped at the configured agent or model limit in affected scenarios.":
    "Die Ausgabe ist in betroffenen Szenarien auf das konfigurierte Agenten- oder Modelllimit begrenzt.",
  "Output tokens per call": "Ausgabetoken pro Aufruf",
  "Output · {count} tokens": "Ausgabe · {count} Token",
  "Quick answer": "Schnelle Antwort",
  Retry: "Erneut versuchen",
  "Starting input tokens": "Anfängliche Eingabetoken",
  "Task assumptions": "Aufgabenannahmen",
  "Tasks per day": "Aufgaben pro Tag",
  "The task choices fill in these assumptions for you. Adjust them if you know how much work your agent usually does.":
    "Die Aufgabenauswahl füllt diese Annahmen aus. Passen Sie sie an, wenn Sie den üblichen Arbeitsumfang Ihres Agenten kennen.",
  "This task is too large for the selected model. Try a smaller task or adjust Advanced assumptions.":
    "Diese Aufgabe ist für das gewählte Modell zu groß. Wählen Sie eine kleinere Aufgabe oder ändern Sie die Erweiterten Annahmen.",
  "Tool result tokens per call": "Werkzeugergebnis-Token pro Aufruf",
  "Using {model}": "Verwendet {model}",
  "We can’t estimate this setup yet. Check the model and the agent’s output limit.":
    "Diese Konfiguration lässt sich noch nicht schätzen. Prüfen Sie das Modell und das Ausgabelimit des Agenten.",
  "We couldn’t load your estimate. Please try again.":
    "Die Schätzung konnte nicht geladen werden. Versuchen Sie es erneut.",
  "We don’t have prices for this model yet. Try another model to see an estimate.":
    "Für dieses Modell liegen noch keine Preise vor. Wählen Sie ein anderes Modell für eine Schätzung.",
  "What does a task look like?": "Wie sieht eine Aufgabe aus?",
  "What will this agent cost?": "Was wird dieser Agent kosten?",
  "Where the cost comes from": "Wie sich die Kosten zusammensetzen",
  "Your message and any starting context.": "Ihre Nachricht und der anfängliche Kontext.",
  "{count} tasks per month": "{count} Aufgaben pro Monat",
  "{range} for lighter to heavier tasks": "{range} für einfachere bis aufwendigere Aufgaben",
  "Enter tasks per day below": "Geben Sie unten die Aufgaben pro Tag ein",
  "A guide, not a fixed price or spending limit. Includes AI usage only. Paid tools, hosting and other service fees are extra. Trying this calculator is free and won’t run or change your agent.":
    "Ein Richtwert, kein Festpreis oder Ausgabenlimit. Nur KI-Nutzung ist enthalten. Kostenpflichtige Werkzeuge, Hosting und andere Dienste kommen hinzu. Der Rechner ist kostenlos und startet oder ändert Ihren Agenten nicht.",
  "Includes roughly {count} tokens of instructions, tools and message overhead on every call. Previous outputs and tool results accumulate in the conversation.":
    "Jeder Aufruf enthält etwa {count} Token für Anweisungen, Werkzeuge und Nachrichtenstruktur. Frühere Ausgaben und Werkzeugergebnisse sammeln sich im Gespräch an.",
  "Planning estimate, not a quote or spending limit. Lighter and heavier scenarios use half to twice the model calls, output and tool results. This is a sensitivity range, not a confidence interval or a maximum.":
    "Eine Planungsschätzung, kein Angebot oder Ausgabenlimit. Leichtere und aufwendigere Szenarien nutzen die halbe bis doppelte Anzahl an Modellaufrufen, Ausgaben und Werkzeugergebnissen. Dies ist eine Sensitivitätsspanne, kein Konfidenzintervall oder Höchstwert.",
  "Uses catalog token prices and approximate text sizing, with no cache discount. Actual costs depend on the task, retries, reasoning, cache reads/writes and provider pricing. Excludes sandbox time, paid tools, media, extra models or subagents, and platform fees.":
    "Verwendet Katalogpreise und eine ungefähre Textgröße ohne Cache-Rabatt. Tatsächliche Kosten hängen von Aufgabe, Wiederholungen, Denkschritten, Cache-Lese-/Schreibvorgängen und Anbieterpreisen ab. Sandbox-Zeit, kostenpflichtige Werkzeuge, Medien, weitere Modelle oder Unteragenten und Plattformgebühren sind ausgeschlossen.",
  "Uses the current agent draft. Estimator inputs do not change the agent. Run representative tasks in Playground and compare their recorded usage before setting a budget.":
    "Verwendet den aktuellen Agentenentwurf. Eingaben ändern den Agenten nicht. Führen Sie vor der Budgetplanung typische Aufgaben im Playground aus und vergleichen Sie die aufgezeichnete Nutzung.",
  "Smart estimate": "Intelligente Schätzung",
  "Manual calculator": "Manueller Rechner",
  "What should we plan for? (optional)": "Was planen wir? (optional)",
  "Infer typical tasks from this agent": "Typische Aufgaben dieses Agenten ableiten",
  "Update estimate": "Schätzung aktualisieren",
  "AI suggests tasks from your agent. Usage sliders update instantly without another AI call.":
    "Die KI schlägt Aufgaben anhand Ihres Agenten vor. Regler aktualisieren die Prognose sofort ohne weiteren KI-Aufruf.",
  "Understanding your agent and estimating typical tasks…":
    "Agent wird analysiert und typische Aufgaben werden geschätzt…",
  "Smart estimates are unavailable right now. Retry or use the manual calculator.":
    "Intelligente Schätzungen sind derzeit nicht verfügbar. Erneut versuchen oder den manuellen Rechner nutzen.",
  "Cost projection": "Kostenprognose",
  "Average per task": "Durchschnitt pro Aufgabe",
  "Daily cost": "Tägliche Kosten",
  "30-day month · Each task starts a new conversation":
    "30-Tage-Monat · Jede Aufgabe beginnt ein neues Gespräch",
  "Check task assumptions and select at least one task. Oversized tasks cannot be priced.":
    "Annahmen prüfen und mindestens eine Aufgabe auswählen. Zu große Aufgaben können nicht berechnet werden.",
  "Usage projection": "Nutzungsprognose",
  "By tasks": "Nach Aufgaben",
  "By users": "Nach Nutzern",
  "Active users": "Aktive Nutzer",
  "Tasks per user per day": "Aufgaben pro Nutzer und Tag",
  "Enter a whole number within the displayed limits.":
    "Eine ganze Zahl innerhalb der angegebenen Grenzen eingeben.",
  "Task mix": "Aufgabenmix",
  "Typical tasks for your agent": "Typische Aufgaben Ihres Agenten",
  "Suggested by AI, not measured usage. Equal mix to start; adjust relative frequency or set a task to zero.":
    "KI-Vorschläge, keine gemessene Nutzung. Anfangs gleich verteilt; relative Häufigkeit anpassen oder eine Aufgabe auf null setzen.",
  Frequency: "Häufigkeit",
  "Monthly spend as usage grows": "Monatskosten bei steigender Nutzung",
  "Monthly planning range: {low} – {high}": "Monatliche Planungsspanne: {low} – {high}",
  "Largest cost driver: {task}": "Größter Kostenfaktor: {task}",
  "Edit task": "Aufgabe bearbeiten",
  "Task example": "Aufgabenbeispiel",
  "Changing the example does not recalculate assumptions. Use the planning input above to generate a new estimate.":
    "Eine Änderung des Beispiels berechnet die Annahmen nicht neu. Für eine neue Schätzung das Planungsfeld oben nutzen.",
  "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Current catalog prices and model limits apply. Cache discounts are not assumed.":
    "Anweisungen und Tools zählen bei jedem Aufruf; frühere Ausgaben und Tool-Ergebnisse summieren sich. Aktuelle Katalogpreise und Modellgrenzen gelten. Cache-Rabatte werden nicht angenommen.",
  "Planning scenarios, not quotes or spending limits. The shaded range varies task complexity, not statistical confidence. AI token costs only: paid tools, hosting, media, other agents and service fees are extra. No tasks are executed and your agent is not changed.":
    "Planungsszenarien, keine Angebote oder Ausgabenlimits. Die Schattierung variiert die Aufgabenkomplexität, nicht die statistische Sicherheit. Nur KI-Tokenkosten: kostenpflichtige Tools, Hosting, Medien, weitere Agenten und Gebühren kommen hinzu. Keine Aufgaben werden ausgeführt, der Agent bleibt unverändert.",
  "{label} slider": "Regler {label}",
  "Monthly cost projection with lighter and heavier task scenarios":
    "Monatskostenprognose für einfachere und aufwendigere Aufgaben",
  "{count} tasks / day": "{count} Aufgaben / Tag",
  "At current usage": "Bei aktueller Nutzung",
  "/ month": "/ Monat",
};
