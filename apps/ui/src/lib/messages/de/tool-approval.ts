// de product copy for tool approval: Settings → Approvals, My approvals, the agent's
// approval settings, the session mode switch and review marks in chat
// (knowledge/execution/tool-approval.md).
export const messages: Readonly<Record<string, string>> = {
  "A reviewer answers first, asks you when unsure":
    "Ein Prüfer antwortet zuerst und fragt Sie, wenn er unsicher ist",
  "A reviewer answers routine calls; you get the unclear ones":
    "Ein Prüfer beantwortet Routineaufrufe; die unklaren landen bei Ihnen",
  Action: "Aktion",
  "Add a note": "Notiz hinzufügen",
  "Add rule": "Regel hinzufügen",
  "Agent default": "Agent-Standard",
  "Allow at least one mode.": "Erlauben Sie mindestens einen Modus.",
  "Allowed modes": "Erlaubte Modi",
  "Always ask": "Immer fragen",
  "Applies from the next tool call.": "Gilt ab dem nächsten Tool-Aufruf.",
  "Applies to sessions you start. You can still switch one session from its header, within what the organization allows.":
    "Gilt für Sitzungen, die Sie starten. Eine einzelne Sitzung können Sie weiterhin in ihrer Kopfzeile umschalten, im Rahmen dessen, was die Organisation erlaubt.",
  "Approval mode": "Genehmigungsmodus",
  "Approval mode: {mode}": "Genehmigungsmodus: {mode}",
  "Approvals: {mode}": "Genehmigungen: {mode}",
  "Asked a person": "Person gefragt",
  "Asks before any change": "Fragt vor jeder Änderung",
  "Asks before risky calls": "Fragt vor riskanten Aufrufen",
  "Auto review is paused after several blocked calls. Every call now waits for you until you approve one.":
    "Die automatische Prüfung ist nach mehreren blockierten Aufrufen pausiert. Jeder Aufruf wartet jetzt auf Sie, bis Sie einen genehmigen.",
  "Auto shows only when the organization allows it.":
    "Auto erscheint nur, wenn die Organisation es erlaubt.",
  "Auto-allowed": "Automatisch erlaubt",
  "Choose which approval modes agents may use and set rules for every agent":
    "Legen Sie fest, welche Genehmigungsmodi Agents verwenden dürfen, und setzen Sie Regeln für alle Agents",
  "Could not change the mode. You may not be allowed to manage this session.":
    "Der Modus konnte nicht geändert werden. Möglicherweise dürfen Sie diese Sitzung nicht verwalten.",
  "Could not load recent reviews.": "Die letzten Prüfungen konnten nicht geladen werden.",
  "Could not save.": "Speichern fehlgeschlagen.",
  "Each agent's own mode": "Der eigene Modus jedes Agents",
  "How agents in this organization get a go-ahead before they use a tool.":
    "Wie Agents in dieser Organisation eine Freigabe erhalten, bevor sie ein Tool verwenden.",
  "How tool calls in your own sessions get approved.":
    "Wie Tool-Aufrufe in Ihren eigenen Sitzungen genehmigt werden.",
  "Modes agents and people may pick. A mode you turn off is raised to the next more careful one you allow.":
    "Modi, die Agents und Personen wählen dürfen. Ein ausgeschalteter Modus wird auf den nächstvorsichtigeren erlaubten angehoben.",
  "My approvals": "Meine Genehmigungen",
  "My extra rules": "Meine zusätzlichen Regeln",
  "Never asks": "Fragt nie",
  "No auto reviews yet.": "Noch keine automatischen Prüfungen.",
  Normal: "Normal",
  "Not available: this agent's manager turned approvals on":
    "Nicht verfügbar: Der Verwalter dieses Agents hat Genehmigungen aktiviert",
  "Note {number}": "Notiz {number}",
  "Notes for the reviewer": "Notizen für den Prüfer",
  "Only organization owners and admins can change these settings.":
    "Nur Eigentümer und Administratoren der Organisation können diese Einstellungen ändern.",
  "Only the prod-* clusters are production.": "Nur die prod-*-Cluster sind Produktion.",
  "Personal rules can only add caution: Always ask or Never. They apply to calls agents make on your behalf.":
    "Persönliche Regeln können nur Vorsicht hinzufügen: Immer fragen oder Nie. Sie gelten für Aufrufe, die Agents in Ihrem Namen machen.",
  "Plain sentences the auto reviewer reads for every agent: your domains, your repositories, which systems are production.":
    "Einfache Sätze, die der automatische Prüfer für jeden Agent liest: Ihre Domains, Ihre Repositories, welche Systeme Produktion sind.",
  "Plain sentences the auto reviewer reads, such as which repositories are yours or what counts as production.":
    "Einfache Sätze, die der automatische Prüfer liest, etwa welche Repositories Ihnen gehören oder was als Produktion gilt.",
  Protective: "Schützend",
  "Recent reviews": "Letzte Prüfungen",
  "Remove note {number}": "Notiz {number} entfernen",
  "Remove rule for {tool}": "Regel für {tool} entfernen",
  Rules: "Regeln",
  "Rules apply in every mode; the strictest match wins. A tool name can use * as a wildcard.":
    "Regeln gelten in jedem Modus; der strengste Treffer gewinnt. Ein Tool-Name kann * als Platzhalter verwenden.",
  "Rules for every agent": "Regeln für alle Agents",
  "Start my sessions in": "Meine Sitzungen starten in",
  "The approval mode your sessions start in and your own extra rules":
    "Der Genehmigungsmodus, in dem Ihre Sitzungen starten, und Ihre eigenen zusätzlichen Regeln",
  "The last calls the auto reviewer decided on, newest first.":
    "Die letzten Aufrufe, über die der automatische Prüfer entschieden hat, neueste zuerst.",
  "These apply to every agent in the organization, in every mode. When rules disagree, the most cautious one wins.":
    "Diese gelten für jeden Agent der Organisation in jedem Modus. Widersprechen sich Regeln, gewinnt die vorsichtigste.",
  "Tool name or pattern": "Tool-Name oder Muster",
  "Trust notes for the reviewer": "Vertrauensnotizen für den Prüfer",
  "Unable to load approval settings.": "Genehmigungseinstellungen konnten nicht geladen werden.",
  Verdict: "Urteil",
  When: "Wann",
  "Why you're asked:": "Warum Sie gefragt werden:",
  "Your own default, else the agent's mode ({mode})":
    "Ihr eigener Standard, sonst der Modus des Agents ({mode})",
  "github.com/acme is ours; pushing branches there is routine.":
    "github.com/acme gehört uns; Branches dorthin zu pushen ist Routine.",
  "{mode} (agent default)": "{mode} (Agent-Standard)",
};
