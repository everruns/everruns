// de product copy for MCP Apps views from connected MCP servers
// (knowledge/integrations/mcp-apps-host.md).
export const messages: Readonly<Record<string, string>> = {
  "The view could not be loaded": "Die Ansicht konnte nicht geladen werden",
  "interactive view from {value0}": "interaktive Ansicht von {value0}",
  "Retry view": "Ansicht erneut laden",
  "{value0} view: {value1}": "{value0}-Ansicht: {value1}",
  "The interactive view didn't load ({value0}). Showing the text result.":
    "Die interaktive Ansicht wurde nicht geladen ({value0}). Das Textergebnis wird angezeigt.",
  "Loading view from {value0}…": "Ansicht von {value0} wird geladen…",
  "Let the {value0} view run {value1}?": "Darf die {value0}-Ansicht {value1} ausführen?",
  "The server does not mark this tool as read-only, so it may change something.":
    "Der Server kennzeichnet dieses Tool nicht als schreibgeschützt, es kann also etwas ändern.",
  "Run tool": "Tool ausführen",
  "Interactive views": "Interaktive Ansichten",
  "On, with a picture and link in Slack": "An, mit Bild und Link in Slack",
  "On, text only in Slack": "An, in Slack nur Text",
  "Show the charts and screens this server provides under its tool calls, in a sandboxed frame. A view reaches only the sites the server declares; tools that are not read-only ask before they run.":
    "Zeigt die Diagramme und Bildschirme dieses Servers unter seinen Tool-Aufrufen in einem abgeschotteten Rahmen. Eine Ansicht erreicht nur die Websites, die der Server angibt; Tools, die nicht schreibgeschützt sind, fragen vor dem Ausführen nach.",
  "the view did not start": "die Ansicht ist nicht gestartet",
};
