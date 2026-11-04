export const messages: Readonly<Record<string, string>> = {
  "External agent": "Externer Agent",
  "Run this agent's turns on your own HTTPS endpoint.":
    "Führen Sie die Turns dieses Agents auf Ihrem eigenen HTTPS-Endpunkt aus.",
  "Your endpoint runs this agent's turns.": "Ihr Endpunkt führt die Turns dieses Agents aus.",
  "Your code runs each turn with its own system prompt, model and tools. Sessions, Playground, Trace and channels keep working. While an endpoint is connected, the page hides the settings it owns.":
    "Ihr Code führt jeden Turn mit eigenem System-Prompt, Modell und eigenen Tools aus. Sitzungen, Playground, Trace und Kanäle funktionieren weiter. Solange ein Endpunkt verbunden ist, blendet die Seite die Einstellungen aus, die er verwaltet.",
  "Your endpoint runs every turn with its own system prompt, model and tools. Everruns keeps the sessions, channels, traces and usage for this agent.":
    "Ihr Endpunkt führt jeden Turn mit eigenem System-Prompt, Modell und eigenen Tools aus. Everruns verwaltet weiterhin Sitzungen, Kanäle, Traces und Nutzung dieses Agents.",
  "Harness, capabilities, model, MCP servers, credentials, files, network access, sandbox and token limits are hidden: your endpoint owns them.":
    "Harness, Fähigkeiten, Modell, MCP-Server, Anmeldedaten, Dateien, Netzwerkzugriff, Sandbox und Token-Limits sind ausgeblendet: Ihr Endpunkt verwaltet sie.",
  "Deploy your endpoint": "Endpunkt bereitstellen",
  "Any HTTPS service that implements the endpoint protocol.":
    "Jeder HTTPS-Dienst, der das Endpunktprotokoll implementiert.",
  "Cloudflare agent guide": "Anleitung zum Cloudflare-Agent",
  "Install an endpoint secret": "Endpunkt-Secret hinterlegen",
  "Set it in your endpoint's environment before you connect, for example with wrangler secret put EVERRUNS_ENDPOINT_SECRET. At least 32 characters. Everruns stores it encrypted and does not show it again.":
    "Legen Sie es vor dem Verbinden in der Umgebung Ihres Endpunkts ab, zum Beispiel mit wrangler secret put EVERRUNS_ENDPOINT_SECRET. Mindestens 32 Zeichen. Everruns speichert es verschlüsselt und zeigt es nicht erneut an.",
  "Endpoint URL": "Endpunkt-URL",
  "Everruns sends a signed challenge to check that the endpoint holds the secret. After you connect, messages and session history for this agent go to the endpoint. Running turns must finish first.":
    "Everruns sendet eine signierte Challenge, um zu prüfen, dass der Endpunkt das Secret kennt. Nach dem Verbinden gehen Nachrichten und Sitzungsverlauf dieses Agents an den Endpunkt. Laufende Turns müssen zuerst abgeschlossen sein.",
  "Verify and connect": "Prüfen und verbinden",
  "Could not connect the endpoint.": "Der Endpunkt konnte nicht verbunden werden.",
  "Could not disconnect the endpoint.": "Der Endpunkt konnte nicht getrennt werden.",
  Disconnected: "Getrennt",
  Queue: "Warteschlange",
  "New messages wait until an endpoint is connected again.":
    "Neue Nachrichten warten, bis wieder ein Endpunkt verbunden ist.",
  "The last call failed": "Der letzte Aufruf ist fehlgeschlagen",
  "Open the session's trace for details.": "Details finden Sie im Trace der Sitzung.",
  "Change the endpoint or rotate the secret": "Endpunkt ändern oder Secret erneuern",
  "Install the new secret in your endpoint first, then verify again. The current connection keeps working until the new one is verified.":
    "Hinterlegen Sie zuerst das neue Secret in Ihrem Endpunkt und prüfen Sie dann erneut. Die aktuelle Verbindung funktioniert, bis die neue geprüft ist.",
  "Update connection": "Verbindung aktualisieren",
  "Disconnect endpoint": "Endpunkt trennen",
  "New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.":
    "Neue Nachrichten warten in der Warteschlange, bis wieder ein Endpunkt verbunden ist. Everruns führt sie nicht selbst aus.",
  "Disconnect the endpoint? New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.":
    "Endpunkt trennen? Neue Nachrichten warten in der Warteschlange, bis wieder ein Endpunkt verbunden ist. Everruns führt sie nicht selbst aus.",
  "Connecting an endpoint requires permission to change dangerous agent settings.":
    "Zum Verbinden eines Endpunkts ist die Berechtigung erforderlich, sensible Agent-Einstellungen zu ändern.",
};
