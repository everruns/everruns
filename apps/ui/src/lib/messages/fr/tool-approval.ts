// fr product copy for tool approval: Settings → Approvals, My approvals, the agent's
// approval settings, the session mode switch and review marks in chat
// (knowledge/execution/tool-approval.md).
export const messages: Readonly<Record<string, string>> = {
  "A reviewer answers first, asks you when unsure":
    "Un réviseur répond d'abord et vous demande en cas de doute",
  "A reviewer answers routine calls; you get the unclear ones":
    "Un réviseur répond aux appels courants ; les cas flous vous reviennent",
  Action: "Action",
  "Add a note": "Ajouter une note",
  "Add rule": "Ajouter une règle",
  "Agent default": "Par défaut de l'agent",
  "Allow at least one mode.": "Autorisez au moins un mode.",
  "Allowed modes": "Modes autorisés",
  "Always ask": "Toujours demander",
  "Applies from the next tool call.": "S'applique dès le prochain appel d'outil.",
  "Applies to sessions you start. You can still switch one session from its header, within what the organization allows.":
    "S'applique aux sessions que vous lancez. Vous pouvez toujours changer une session depuis son en-tête, dans les limites autorisées par l'organisation.",
  "Approval mode": "Mode d'approbation",
  "Approval mode: {mode}": "Mode d'approbation : {mode}",
  "Approvals: {mode}": "Approbations : {mode}",
  "Asked a person": "Personne sollicitée",
  "Asks before any change": "Demande avant toute modification",
  "Asks before risky calls": "Demande avant les appels risqués",
  "Auto review is paused after several blocked calls. Every call now waits for you until you approve one.":
    "La révision automatique est en pause après plusieurs appels bloqués. Chaque appel vous attend désormais jusqu'à ce que vous en approuviez un.",
  "Auto shows only when the organization allows it.":
    "Auto n'apparaît que si l'organisation l'autorise.",
  "Auto-allowed": "Autorisé automatiquement",
  "Choose which approval modes agents may use and set rules for every agent":
    "Choisissez les modes d'approbation que les agents peuvent utiliser et définissez des règles pour tous les agents",
  "Could not change the mode. You may not be allowed to manage this session.":
    "Impossible de changer le mode. Vous n'êtes peut-être pas autorisé à gérer cette session.",
  "Could not load recent reviews.": "Impossible de charger les révisions récentes.",
  "Could not save.": "Enregistrement impossible.",
  "Each agent's own mode": "Le mode propre à chaque agent",
  "How agents in this organization get a go-ahead before they use a tool.":
    "Comment les agents de cette organisation obtiennent un feu vert avant d'utiliser un outil.",
  "How tool calls in your own sessions get approved.":
    "Comment les appels d'outils de vos propres sessions sont approuvés.",
  "Modes agents and people may pick. A mode you turn off is raised to the next more careful one you allow.":
    "Modes que les agents et les personnes peuvent choisir. Un mode désactivé est relevé au mode autorisé suivant, plus prudent.",
  "My approvals": "Mes approbations",
  "My extra rules": "Mes règles supplémentaires",
  "Never asks": "Ne demande jamais",
  "No auto reviews yet.": "Aucune révision automatique pour l'instant.",
  Normal: "Normal",
  "Not available: this agent's manager turned approvals on":
    "Indisponible : le responsable de cet agent a activé les approbations",
  "Note {number}": "Note {number}",
  "Notes for the reviewer": "Notes pour le réviseur",
  "Only organization owners and admins can change these settings.":
    "Seuls les propriétaires et les administrateurs de l'organisation peuvent modifier ces paramètres.",
  "Only the prod-* clusters are production.": "Seuls les clusters prod-* sont en production.",
  "Personal rules can only add caution: Always ask or Never. They apply to calls agents make on your behalf.":
    "Les règles personnelles ne peuvent qu'ajouter de la prudence : Toujours demander ou Jamais. Elles s'appliquent aux appels que les agents font en votre nom.",
  "Plain sentences the auto reviewer reads for every agent: your domains, your repositories, which systems are production.":
    "Des phrases simples que le réviseur automatique lit pour chaque agent : vos domaines, vos dépôts, quels systèmes sont en production.",
  "Plain sentences the auto reviewer reads, such as which repositories are yours or what counts as production.":
    "Des phrases simples que lit le réviseur automatique, par exemple quels dépôts sont les vôtres ou ce qui compte comme production.",
  Protective: "Protecteur",
  "Recent reviews": "Révisions récentes",
  "Remove note {number}": "Supprimer la note {number}",
  "Remove rule for {tool}": "Supprimer la règle pour {tool}",
  Rules: "Règles",
  "Rules apply in every mode; the strictest match wins. A tool name can use * as a wildcard.":
    "Les règles s'appliquent dans tous les modes ; la correspondance la plus stricte l'emporte. Un nom d'outil peut utiliser * comme joker.",
  "Rules for every agent": "Règles pour tous les agents",
  "Start my sessions in": "Démarrer mes sessions en",
  "The approval mode your sessions start in and your own extra rules":
    "Le mode d'approbation de départ de vos sessions et vos propres règles supplémentaires",
  "The last calls the auto reviewer decided on, newest first.":
    "Les derniers appels sur lesquels le réviseur automatique a statué, les plus récents d'abord.",
  "These apply to every agent in the organization, in every mode. When rules disagree, the most cautious one wins.":
    "Elles s'appliquent à chaque agent de l'organisation, dans tous les modes. En cas de désaccord, la règle la plus prudente l'emporte.",
  "Tool name or pattern": "Nom d'outil ou motif",
  "Trust notes for the reviewer": "Notes de confiance pour le réviseur",
  "Unable to load approval settings.": "Impossible de charger les paramètres d'approbation.",
  Verdict: "Verdict",
  When: "Quand",
  "Why you're asked:": "Pourquoi on vous demande :",
  "Your own default, else the agent's mode ({mode})":
    "Votre propre défaut, sinon le mode de l'agent ({mode})",
  "github.com/acme is ours; pushing branches there is routine.":
    "github.com/acme est à nous ; y pousser des branches est courant.",
  "{mode} (agent default)": "{mode} (par défaut de l'agent)",
};
