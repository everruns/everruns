// de product copy for the agent's Channels and triggers tab, Pause agent and the GitHub page.
export const messages: Readonly<Record<string, string>> = {
  "Channels and triggers": "Kanäle und Trigger",
  "How people and software reach this agent, and when it starts on its own.":
    "Wie Menschen und Software diesen Agenten erreichen und wann er von selbst startet.",
  "When this agent starts on its own: schedules, webhooks, GitHub events and MCP server events.":
    "Wann dieser Agent von selbst startet: Zeitpläne, Webhooks, GitHub-Ereignisse und MCP-Server-Ereignisse.",
  "Add a schedule, a webhook or an event to start this agent on its own.":
    "Fügen Sie einen Zeitplan, einen Webhook oder ein Ereignis hinzu, damit dieser Agent von selbst startet.",
  "Delete this trigger? It stops starting the agent.":
    "Diesen Trigger löschen? Er startet den Agenten dann nicht mehr.",
  "Turn off": "Ausschalten",
  "Turn on": "Einschalten",
  "Pause agent": "Agent pausieren",
  "Resume agent": "Agent fortsetzen",
  "Pause {value0}? Its channels stop accepting messages and its triggers and schedules stop starting it until you resume it. Sessions already running continue.":
    "{value0} pausieren? Seine Kanäle nehmen keine Nachrichten mehr an und seine Trigger und Zeitpläne starten ihn nicht, bis Sie ihn fortsetzen. Laufende Sitzungen werden fortgesetzt.",
  "This agent is paused": "Dieser Agent ist pausiert",
  "Its channels do not accept messages and its triggers and schedules do not start it. Each channel and trigger keeps its own setting for when you resume.":
    "Seine Kanäle nehmen keine Nachrichten an und seine Trigger und Zeitpläne starten ihn nicht. Jeder Kanal und jeder Trigger behält seine eigene Einstellung für das Fortsetzen.",
  "Publish opens this channel to callers and Unpublish closes it. Pausing the agent overrides both.":
    "Veröffentlichen öffnet diesen Kanal für Aufrufer, Zurückziehen schließt ihn. Das Pausieren des Agenten hat Vorrang vor beidem.",
  "Add a channel from an agent's Channels and triggers tab. New channels start as drafts and take no traffic until you publish them.":
    "Fügen Sie einen Kanal über die Registerkarte „Kanäle und Trigger“ eines Agenten hinzu. Neue Kanäle beginnen als Entwürfe und nehmen keinen Traffic auf, bis Sie sie veröffentlichen.",
  "Need another system to start the agent with an HTTP request?":
    "Soll ein anderes System den Agenten mit einer HTTP-Anfrage starten?",
  "Add a webhook trigger": "Webhook-Trigger hinzufügen",
  "Trigger type": "Trigger-Typ",
  Token: "Token",
  Generate: "Erzeugen",
  "Leave empty to keep the current token": "Leer lassen, um das aktuelle Token zu behalten",
  "Copy it now: it is not shown again. Callers send it as Authorization: Bearer <token> or X-Everruns-Webhook-Token.":
    "Kopieren Sie es jetzt: Es wird nicht noch einmal angezeigt. Aufrufer senden es als Authorization: Bearer <token> oder X-Everruns-Webhook-Token.",
  "Recent runs": "Letzte Läufe",
  "Loading runs…": "Läufe werden geladen…",
  "Starts when {value0} sends {value1}.": "Startet, wenn {value0} {value1} sendet.",
  "MCP event triggers are created and changed through the API and CLI.":
    "MCP-Ereignis-Trigger werden über die API und die CLI erstellt und geändert.",
  "Who {value0} is on GitHub when it reads code and comments on pull requests.":
    "Wer {value0} auf GitHub ist, wenn er Code liest und Pull Requests kommentiert.",
  "Not connected": "Nicht verbunden",
  "Not installed": "Nicht installiert",
  "Own GitHub App": "Eigene GitHub App",
  "Shared Everruns App": "Gemeinsame Everruns App",
  Kind: "Art",
  trigger: "Trigger",
  "The agent's GitHub tools, while the GitHub capability is on":
    "Die GitHub-Werkzeuge des Agenten, solange die GitHub-Fähigkeit aktiv ist",
  "It may have been deleted, or you may not have access to it.":
    "Er wurde möglicherweise gelöscht, oder Sie haben keinen Zugriff darauf.",
};
