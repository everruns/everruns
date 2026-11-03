// fr product copy for the agent's Channels and triggers tab, Pause agent and the GitHub page.
export const messages: Readonly<Record<string, string>> = {
  "Channels and triggers": "Canaux et déclencheurs",
  "How people and software reach this agent, and when it starts on its own.":
    "Comment les personnes et les logiciels joignent cet agent, et quand il démarre de lui-même.",
  "When this agent starts on its own: schedules, webhooks, GitHub events and MCP server events.":
    "Quand cet agent démarre de lui-même : planifications, webhooks, événements GitHub et événements de serveurs MCP.",
  "Add a schedule, a webhook or an event to start this agent on its own.":
    "Ajoutez une planification, un webhook ou un événement pour que cet agent démarre de lui-même.",
  "Delete this trigger? It stops starting the agent.":
    "Supprimer ce déclencheur ? Il ne démarrera plus l’agent.",
  "Turn off": "Désactiver",
  "Turn on": "Activer",
  "Pause agent": "Mettre l’agent en pause",
  "Resume agent": "Reprendre l’agent",
  "Pause {value0}? Its channels stop accepting messages and its triggers and schedules stop starting it until you resume it. Sessions already running continue.":
    "Mettre {value0} en pause ? Ses canaux n’acceptent plus de messages et ses déclencheurs et planifications ne le démarrent plus jusqu’à la reprise. Les sessions en cours continuent.",
  "This agent is paused": "Cet agent est en pause",
  "Its channels do not accept messages and its triggers and schedules do not start it. Each channel and trigger keeps its own setting for when you resume.":
    "Ses canaux n’acceptent pas de messages et ses déclencheurs et planifications ne le démarrent pas. Chaque canal et déclencheur garde son propre réglage pour la reprise.",
  "Publish opens this channel to callers and Unpublish closes it. Pausing the agent overrides both.":
    "Publier ouvre ce canal aux appelants et Dépublier le ferme. La mise en pause de l’agent prime sur les deux.",
  "Add a channel from an agent's Channels and triggers tab. New channels start as drafts and take no traffic until you publish them.":
    "Ajoutez un canal depuis l’onglet Canaux et déclencheurs d’un agent. Les nouveaux canaux démarrent en brouillon et ne reçoivent aucun trafic tant que vous ne les publiez pas.",
  "Need another system to start the agent with an HTTP request?":
    "Besoin qu’un autre système démarre l’agent par une requête HTTP ?",
  "Add a webhook trigger": "Ajouter un déclencheur webhook",
  "Trigger type": "Type de déclencheur",
  Token: "Jeton",
  Generate: "Générer",
  "Leave empty to keep the current token": "Laisser vide pour conserver le jeton actuel",
  "Copy it now: it is not shown again. Callers send it as Authorization: Bearer <token> or X-Everruns-Webhook-Token.":
    "Copiez-le maintenant : il ne sera plus affiché. Les appelants l’envoient comme Authorization: Bearer <token> ou X-Everruns-Webhook-Token.",
  "Recent runs": "Exécutions récentes",
  "Loading runs…": "Chargement des exécutions…",
  "Starts when {value0} sends {value1}.": "Démarre quand {value0} envoie {value1}.",
  "MCP event triggers are created and changed through the API and CLI.":
    "Les déclencheurs d’événements MCP se créent et se modifient par l’API et la CLI.",
  "Who {value0} is on GitHub when it reads code and comments on pull requests.":
    "Qui est {value0} sur GitHub quand il lit du code et commente des pull requests.",
  "Not connected": "Non connecté",
  "Not installed": "Non installée",
  "Own GitHub App": "Sa propre app GitHub",
  "Shared Everruns App": "App Everruns partagée",
  Kind: "Type",
  trigger: "déclencheur",
  "The agent's GitHub tools, while the GitHub capability is on":
    "Les outils GitHub de l’agent, tant que la capacité GitHub est active",
  "It may have been deleted, or you may not have access to it.":
    "Il a peut-être été supprimé, ou vous n’y avez pas accès.",
};
