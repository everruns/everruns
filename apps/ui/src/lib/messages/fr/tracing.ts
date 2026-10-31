// Product-owned tracing copy. Keep credentials and authored destination names outside translation.
export const messages: Readonly<Record<string, string>> = {
  Tracing: "Traçage",
  "Add destination": "Ajouter une destination",
  "Edit tracing destination": "Modifier la destination de traçage",
  "Add tracing destination": "Ajouter une destination de traçage",
  Destination: "Destination",
  "Destination name": "Nom de la destination",
  Content: "Contenu",
  Delivery: "Transmission",
  "No tracing destinations": "Aucune destination de traçage",
  "Send agent traces to your Braintrust projects and OTLP collectors.":
    "Envoyez les traces des agents vers vos projets Braintrust et vos collecteurs OTLP.",
  "Connect Braintrust or an OpenTelemetry collector, then choose which agents export traces.":
    "Connectez Braintrust ou un collecteur OpenTelemetry, puis choisissez les agents qui exportent des traces.",
  "Loading tracing…": "Chargement du traçage…",
  "Could not load tracing": "Impossible de charger le traçage",
  "Could not save tracing": "Impossible d’enregistrer le traçage",
  "Could not save destination": "Impossible d’enregistrer la destination",
  "Tracing operation failed": "Échec de l’opération de traçage",
  "Organization defaults": "Valeurs par défaut de l’organisation",
  "Agents inherit these destinations unless they customize routing. Personal Chat is excluded.":
    "Les agents héritent de ces destinations, sauf s’ils personnalisent le routage. Le chat personnel est exclu.",
  "Platform telemetry is configured separately. Exports are best effort; delivery failures do not stop agents.":
    "La télémétrie de la plateforme se configure séparément. Les exports sont effectués au mieux ; les échecs de transmission n’arrêtent pas les agents.",
  "Destination created. Choose defaults or assign it from an agent's Tracing settings.":
    "Destination créée. Choisissez les valeurs par défaut ou affectez-la dans les paramètres de traçage d’un agent.",
  "Metadata only": "Métadonnées uniquement",
  "Messages and tools": "Messages et outils",
  Paused: "En pause",
  "Delivery failing": "Échec de transmission",
  "Export accepted": "Export accepté",
  "No exports yet": "Aucun export pour le moment",
  "Dropped events: {value0}": "Événements abandonnés : {value0}",
  Manage: "Gérer",
  "Edit destination": "Modifier la destination",
  "Send test trace": "Envoyer une trace de test",
  "Synthetic trace accepted by receiver": "Trace synthétique acceptée par le destinataire",
  "Test export failed; review destination delivery status":
    "Échec de l’export de test ; vérifiez l’état de transmission de la destination",
  "Pause destination": "Suspendre la destination",
  "Resume destination": "Reprendre la destination",
  "Delete destination": "Supprimer la destination",
  "Edit defaults": "Modifier les valeurs par défaut",
  "View defaults": "Voir les valeurs par défaut",
  "Organization health": "État de l’organisation",
  "Remove this destination from all routing policies first. Existing exports at the provider are not deleted.":
    "Retirez d’abord cette destination de toutes les règles de routage. Les exports existants chez le fournisseur ne sont pas supprimés.",
  "Credentials stay encrypted and are never exposed to agents or Chat.":
    "Les identifiants restent chiffrés et ne sont jamais exposés aux agents ni au chat.",
  Endpoint: "Point de terminaison",
  "HTTPS only. Private collectors must be reachable through the deployment's approved network boundary.":
    "HTTPS uniquement. Les collecteurs privés doivent être accessibles via le périmètre réseau approuvé du déploiement.",
  "Project ID": "Identifiant du projet",
  "Replace authentication": "Remplacer l’authentification",
  "Authentication header": "En-tête d’authentification",
  "Header value (optional)": "Valeur de l’en-tête (facultative)",
  "API key": "Clé API",
  "Include messages and tool content": "Inclure les messages et le contenu des outils",
  "Prompts, tool results, and detailed errors will leave Everruns.":
    "Les prompts, les résultats des outils et les erreurs détaillées seront transmis hors d’Everruns.",
  "Include reasoning content": "Inclure le contenu du raisonnement",
  "Starts with metadata only. Creating a destination does not assign it to agents.":
    "Commence avec les métadonnées uniquement. Créer une destination ne l’affecte pas aux agents.",
  "Create destination": "Créer une destination",
  "Organization admins manage trace exports.":
    "Les administrateurs de l’organisation gèrent les exports de traces.",
  Routing: "Routage",
  "Use organization defaults": "Utiliser les valeurs par défaut de l’organisation",
  "Add to organization defaults": "Ajouter aux valeurs par défaut de l’organisation",
  "Use selected destinations": "Utiliser les destinations sélectionnées",
  "Turn off external tracing": "Désactiver le traçage externe",
  "Selected destinations": "Destinations sélectionnées",
  "Connect a destination before selecting defaults.":
    "Connectez une destination avant de sélectionner les valeurs par défaut.",
  "Effective destinations": "Destinations effectives",
  "No external destinations.": "Aucune destination externe.",
  "Destination unavailable": "Destination indisponible",
  Inherited: "Hérité",
  "Agent selection": "Sélection de l’agent",
  "Changes apply to new turns in existing and new sessions. Personal Chat is excluded.":
    "Les modifications s’appliquent aux nouveaux tours des sessions existantes et nouvelles. Le chat personnel est exclu.",
  "Routing changed elsewhere. Discard this draft and reload before saving.":
    "Le routage a été modifié ailleurs. Abandonnez ce brouillon et rechargez les données avant d’enregistrer.",
  "Manage organization destinations": "Gérer les destinations de l’organisation",
  "External destinations for this agent’s traces.":
    "Destinations externes pour les traces de cet agent.",
  "Tracing changes apply to new turns after Save.":
    "Les modifications du traçage s’appliquent aux nouveaux tours après l’enregistrement.",
  Destinations: "Destinations",
  "Manage agent trace exports": "Gérer les exports de traces des agents",
  "Organization default": "Valeur par défaut de l’organisation",
  "Agent selections: {value0}": "Sélections des agents : {value0}",
  Unassigned: "Non affecté",
};
