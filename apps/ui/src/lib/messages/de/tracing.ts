// Product-owned tracing copy. Keep credentials and authored destination names outside translation.
export const messages: Readonly<Record<string, string>> = {
  Tracing: "Tracing",
  "Add destination": "Ziel hinzufügen",
  "Edit tracing destination": "Tracing-Ziel bearbeiten",
  "Add tracing destination": "Tracing-Ziel hinzufügen",
  Destination: "Ziel",
  "Destination name": "Zielname",
  Content: "Inhalt",
  Delivery: "Zustellung",
  "No tracing destinations": "Keine Tracing-Ziele",
  "Send agent traces to your Braintrust projects and OTLP collectors.":
    "Senden Sie Agenten-Traces an Ihre Braintrust-Projekte und OTLP-Kollektoren.",
  "Connect Braintrust or an OpenTelemetry collector, then choose which agents export traces.":
    "Verbinden Sie Braintrust oder einen OpenTelemetry-Kollektor und wählen Sie dann die Agenten aus, die Traces exportieren.",
  "Loading tracing…": "Tracing wird geladen…",
  "Could not load tracing": "Tracing konnte nicht geladen werden",
  "Could not save tracing": "Tracing konnte nicht gespeichert werden",
  "Could not save destination": "Ziel konnte nicht gespeichert werden",
  "Tracing operation failed": "Tracing-Vorgang fehlgeschlagen",
  "Organization defaults": "Organisationsvorgaben",
  "Agents inherit these destinations unless they customize routing. Personal Chat is excluded.":
    "Agenten übernehmen diese Ziele, sofern sie das Routing nicht anpassen. Persönlicher Chat ist ausgenommen.",
  "Platform telemetry is configured separately. Exports are best effort; delivery failures do not stop agents.":
    "Die Plattform-Telemetrie wird separat konfiguriert. Exporte erfolgen ohne Zustellgarantie; Zustellfehler halten Agenten nicht an.",
  "Destination created. Choose defaults or assign it from an agent's Tracing settings.":
    "Ziel erstellt. Wählen Sie Vorgaben aus oder weisen Sie es in den Tracing-Einstellungen eines Agenten zu.",
  "Metadata only": "Nur Metadaten",
  "Messages and tools": "Nachrichten und Werkzeuge",
  Paused: "Pausiert",
  "Delivery failing": "Zustellfehler",
  "Export accepted": "Export angenommen",
  "No exports yet": "Noch keine Exporte",
  "Dropped events: {value0}": "Verworfene Ereignisse: {value0}",
  Manage: "Verwalten",
  "Edit destination": "Ziel bearbeiten",
  "Send test trace": "Test-Trace senden",
  "Synthetic trace accepted by receiver": "Synthetischer Trace vom Empfänger angenommen",
  "Test export failed; review destination delivery status":
    "Testexport fehlgeschlagen; prüfen Sie den Zustellstatus des Ziels",
  "Pause destination": "Ziel pausieren",
  "Resume destination": "Ziel fortsetzen",
  "Delete destination": "Ziel löschen",
  "Edit defaults": "Vorgaben bearbeiten",
  "View defaults": "Vorgaben anzeigen",
  "Organization health": "Organisationszustand",
  "Remove this destination from all routing policies first. Existing exports at the provider are not deleted.":
    "Entfernen Sie dieses Ziel zuerst aus allen Routing-Richtlinien. Bestehende Exporte beim Anbieter werden nicht gelöscht.",
  "Credentials stay encrypted and are never exposed to agents or Chat.":
    "Zugangsdaten bleiben verschlüsselt und werden Agenten oder Chat niemals offengelegt.",
  Endpoint: "Endpunkt",
  "HTTPS only. Private collectors must be reachable through the deployment's approved network boundary.":
    "Nur HTTPS. Private Kollektoren müssen über die für das Deployment zugelassene Netzwerkgrenze erreichbar sein.",
  "Project ID": "Projekt-ID",
  "Replace authentication": "Authentifizierung ersetzen",
  "Authentication header": "Authentifizierungsheader",
  "Header value (optional)": "Header-Wert (optional)",
  "API key": "API-Schlüssel",
  "Include messages and tool content": "Nachrichten und Werkzeuginhalte einschließen",
  "Prompts, tool results, and detailed errors will leave Everruns.":
    "Prompts, Werkzeugergebnisse und detaillierte Fehlermeldungen verlassen Everruns.",
  "Include reasoning content": "Reasoning-Inhalte einschließen",
  "Starts with metadata only. Creating a destination does not assign it to agents.":
    "Beginnt nur mit Metadaten. Das Erstellen eines Ziels weist es keinen Agenten zu.",
  "Create destination": "Ziel erstellen",
  "Organization admins manage trace exports.":
    "Organisationsadministratoren verwalten Trace-Exporte.",
  Routing: "Routing",
  "Use organization defaults": "Organisationsvorgaben verwenden",
  "Add to organization defaults": "Zu Organisationsvorgaben hinzufügen",
  "Use selected destinations": "Ausgewählte Ziele verwenden",
  "Turn off external tracing": "Externes Tracing deaktivieren",
  "Selected destinations": "Ausgewählte Ziele",
  "Connect a destination before selecting defaults.":
    "Verbinden Sie ein Ziel, bevor Sie Vorgaben auswählen.",
  "Effective destinations": "Wirksame Ziele",
  "No external destinations.": "Keine externen Ziele.",
  "Destination unavailable": "Ziel nicht verfügbar",
  Inherited: "Geerbt",
  "Agent selection": "Agentenauswahl",
  "Changes apply to new turns in existing and new sessions. Personal Chat is excluded.":
    "Änderungen gelten für neue Durchläufe in bestehenden und neuen Sitzungen. Persönlicher Chat ist ausgenommen.",
  "Routing changed elsewhere. Discard this draft and reload before saving.":
    "Das Routing wurde an anderer Stelle geändert. Verwerfen Sie diesen Entwurf und laden Sie vor dem Speichern neu.",
  "Manage organization destinations": "Organisationsziele verwalten",
  "External destinations for this agent’s traces.": "Externe Ziele für die Traces dieses Agenten.",
  "Tracing changes apply to new turns after Save.":
    "Tracing-Änderungen gelten nach dem Speichern für neue Durchläufe.",
  Destinations: "Ziele",
  "Manage agent trace exports": "Agenten-Trace-Exporte verwalten",
  "Organization default": "Organisationsvorgabe",
  "Agent selections: {value0}": "Agentenauswahlen: {value0}",
  Unassigned: "Nicht zugewiesen",
};
