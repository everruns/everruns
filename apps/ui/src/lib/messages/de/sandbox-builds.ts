export const messages: Readonly<Record<string, string>> = {
  "Build snapshot": "Snapshot erstellen",
  "Run an install script once, then reuse the resulting provider snapshot.":
    "Installationsskript einmal ausführen und den erstellten Provider-Snapshot wiederverwenden.",
  Provider: "Provider",
  "Base snapshot": "Basis-Snapshot",
  "Provider default": "Provider-Standard",
  "Source revision": "Quellrevision",
  "Select and save a named Organization provider account before building.":
    "Vor dem Erstellen ein benanntes Provider-Konto der Organisation auswählen und speichern.",
  "Save your configuration changes before building.":
    "Konfigurationsänderungen vor dem Erstellen speichern.",
  "Install script": "Installationsskript",
  "Verification and build access": "Überprüfung und Build-Zugriff",
  "Verification command (optional)": "Prüfbefehl (optional)",
  "Metadata file (optional)": "Metadatendatei (optional)",
  "Capture up to 16 KiB of non-secret software notes written by your script.":
    "Bis zu 16 KiB nicht geheimer Softwarehinweise aus dem Skript erfassen.",
  "Total build timeout (seconds)": "Gesamtzeitlimit für den Build (Sekunden)",
  "Build network": "Build-Netzwerk",
  "Loading…": "Wird geladen…",
  "Build result": "Build-Ergebnis",
  Step: "Schritt",
  "Resource cleanup is pending and will be retried.":
    "Die Ressourcenbereinigung steht aus und wird erneut versucht.",
  "Captured software notes": "Erfasste Softwarehinweise",
  "Build log": "Build-Protokoll",
  "Waiting for output…": "Warten auf Ausgabe…",
  "Save new revision": "Neue Revision speichern",
  "Discard snapshot": "Snapshot verwerfen",
  "The source revision changed. Build again from the current template.":
    "Die Quellrevision hat sich geändert. Erneut mit der aktuellen Vorlage erstellen.",
  Queued: "In Warteschlange",
  Running: "Wird ausgeführt",
  Ready: "Bereit",
  Failed: "Fehlgeschlagen",
  Cancelled: "Abgebrochen",
  Saved: "Gespeichert",
  Discarded: "Verworfen",
  "Starting build compute": "Build-Umgebung wird gestartet",
  "Running install script": "Installationsskript wird ausgeführt",
  "Verifying installation": "Installation wird überprüft",
  "Creating snapshot": "Snapshot wird erstellt",
  "Verifying fresh snapshot": "Neuer Snapshot wird überprüft",
  "Cancellation requested": "Abbruch angefordert",
  "Cancel build": "Build abbrechen",
  "Built snapshot": "Erstellter Snapshot",
  "Sandbox Template sections": "Bereiche der Sandbox-Vorlage",
  Configuration: "Konfiguration",
  "Snapshot builds currently support Daytona. Other template targets remain configurable.":
    "Snapshot-Builds unterstützen derzeit Daytona. Andere Vorlagenziele bleiben konfigurierbar.",
  "Runs in fresh isolated build compute. Launch bootstrap commands stay separate. Do not put credentials in scripts.":
    "Wird in einer neuen isolierten Build-Umgebung ausgeführt. Startbefehle bleiben getrennt. Keine Zugangsdaten in Skripte schreiben.",
  "Runs after installation and again on a fresh sandbox from the snapshot. Only what this command checks is verified.":
    "Wird nach der Installation und erneut in einer neuen Sandbox aus dem Snapshot ausgeführt. Nur die Prüfungen dieses Befehls werden bestätigt.",
  "Allow package sources during installation without expanding the finished sandbox’s runtime policy.":
    "Paketquellen während der Installation zulassen, ohne die Laufzeitrichtlinie der fertigen Sandbox zu erweitern.",
  "Build compute and snapshot storage are billed to the selected provider account. Runtime credentials and private workspace data are not attached.":
    "Build-Umgebung und Snapshot-Speicher werden dem ausgewählten Provider-Konto berechnet. Laufzeit-Zugangsdaten und private Arbeitsbereichsdaten werden nicht eingebunden.",
  "Author-supplied reference data. These notes do not grant capabilities or change runtime policy.":
    "Vom Autor bereitgestellte Referenzdaten. Diese Hinweise gewähren keine Fähigkeiten und ändern keine Laufzeitrichtlinien.",
  "Discard this unused provider snapshot? Applied template revisions are kept.":
    "Diesen ungenutzten Provider-Snapshot verwerfen? Angewendete Vorlagenrevisionen bleiben erhalten.",
  "Older builds": "Ältere Builds",
  "Reuse script": "Skript wiederverwenden",
};
