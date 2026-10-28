// Product-owned copy. Preserve interpolation tokens and stable identifiers.
export const messages: Readonly<Record<string, string>> = {
  "Select an agent": "Sélectionnez un agent",
  "Select at least two runs from the eval’s Runs tab to compare.":
    "Sélectionnez au moins deux exécutions dans l'onglet Exécutions de l'évaluation à comparer.",
  "Select runs to compare them case by case.":
    "Sélectionnez les analyses pour les comparer au cas par cas.",
  "Service failing - calls are rejected immediately":
    "Échec du service - les appels sont immédiatement rejetés",
  "Session tags": "Balises de session",
  Share: "Partager",
  Skipped: "Ignoré",
  "Some runs could not be loaded. Check the selected runs and try again.":
    "Certaines exécutions n'ont pas pu être chargées. Vérifiez les exécutions sélectionnées et réessayez.",
  standalone: "autonome",
  "Start your first run": "Commencez votre première course",
  "Starter templates": "Modèles de démarrage",
  "Starting...": "Démarrage...",
  Statistics: "Statistiques",
  "Success Count": "Nombre de réussites",
  Successful: "Réussi",
  "System Load": "Charge du système",
  "Tags (optional)": "Balises (facultatif)",
  "Target Input": "Entrée cible",
  "Target Input (JSON)": "Entrée cible (JSON)",
  "Target Name": "Nom de la cible",
  "Target override:": "Remplacement de la cible :",
  "Target Type": "Type de cible",
  task: "tâche",
  "Task completions and failures per interval (last 15 min)":
    "Achèvements et échecs de tâches par intervalle (15 dernières minutes)",
  "Task Queue": "File d'attente des tâches",
  "Tasks that exhausted all retry attempts. Requeue to retry or delete to discard.":
    "Tâches qui ont épuisé toutes les tentatives de nouvelle tentative. Remettre en file d'attente pour réessayer ou supprimer pour supprimer.",
  "Tasks that have exhausted all retry attempts":
    "Tâches pour lesquelles toutes les tentatives ont été épuisées",
  "Test case name": "Nom du scénario de test",
  "Testing recovery - allowing one call to check if service is back":
    "Test de récupération - permettant à un appel de vérifier si le service est de retour",
  Text: "Texte",
  "The durable queues API is not available.":
    "L'API des files d'attente durables n'est pas disponible.",
  "The durable schedules API is not available.":
    "L'API des planifications durables n'est pas disponible.",
  "The durable workers API is not available. Please ensure the backend is running.":
    "L'API des travailleurs durables n'est pas disponible. Veuillez vous assurer que le backend est en cours d'exécution.",
  "The durable workflows API is not available.":
    "L'API des workflows durables n'est pas disponible.",
  "The schedule could not be found or the API is not available.":
    "Le planning est introuvable ou l'API n'est pas disponible.",
  "The task queue is empty.": "La file d'attente des tâches est vide.",
  "The task queue is empty. Tasks will appear here when workflows schedule activities.":
    "La file d'attente des tâches est vide. Les tâches apparaîtront ici lorsque les flux de travail planifient des activités.",
  "The workflow could not be loaded. It may not exist or the API is unavailable.":
    "Le workflow n'a pas pu être chargé. Il se peut qu'elle n'existe pas ou que l'API ne soit pas disponible.",
  "This eval may have been deleted, moved to another organization, or the URL may be wrong.":
    "Cette évaluation a peut-être été supprimée, déplacée vers une autre organisation ou l'URL est peut-être erronée.",
  "This eval run may have been deleted, moved to another organization, or the URL may be wrong.":
    "Cette exécution d'évaluation a peut-être été supprimée, déplacée vers une autre organisation ou l'URL est peut-être erronée.",
  "This feature is currently disabled.": "Cette fonctionnalité est actuellement désactivée.",
  "This observer may have been deleted, moved to another organization, or the URL may be wrong.":
    "Cet observateur a peut-être été supprimé, déplacé vers une autre organisation ou l'URL est peut-être erronée.",
  "This schedule hasn't been triggered yet. Click 'Trigger' to run it manually.":
    "Ce planning n'a pas encore été déclenché. Cliquez sur « Déclencheur » pour l'exécuter manuellement.",
  'This will delete the circuit breaker state for "':
    "Cela supprimera l'état du disjoncteur pour \"",
  'This will immediately block all calls through the "':
    'Cela bloquera immédiatement tous les appels via le "',
  Throughput: "Débit",
  "Throughput per interval: started, completed, failed (last 15 min)":
    "Débit par intervalle : démarré, terminé, échoué (15 dernières minutes)",
  "Timeline of workflow events": "Chronologie des événements de workflow",
  "Timeout:": "Délai d'expiration :",
  "to immediately allow calls after a service recovers.":
    "pour autoriser immédiatement les appels après la récupération d'un service.",
  "to proactively block calls when you know a service has issues. Use":
    "pour bloquer les appels de manière proactive lorsque vous savez qu'un service a des problèmes. Utiliser",
  "Tool calls ({value0})": "Appels d'outil ({value0})",
  "Tool-level scoring": "Notation au niveau de l'outil",
  Total: "Total",
  "Total Executions": "Total des exécutions",
  "Trends are computed from recent sampled scores; full aggregation is a follow-up.":
    "Les tendances sont calculées à partir des scores récents échantillonnés ; l'agrégation complète est un suivi.",
  Trigger: "Déclencheur",
  "Trigger Now": "Déclenchez maintenant",
  "Triggered by:": "Déclenché par :",
  "Unable to Load Queues": "Impossible de charger les files d'attente",
  "Unable to Load Schedules": "Impossible de charger les programmes",
  "Unable to Load Workers": "Impossible de charger les travailleurs",
  "Unable to Load Workflows": "Impossible de charger les flux de travail",
  "Under backpressure": "Sous contre-pression",
  Unlimited: "Illimité",
  "Update the schedule configuration.": "Mettez à jour la configuration du planning.",
  Use: "Utilisation",
  "Use agent default": "Utiliser les valeurs par défaut de l'agent",
  "Use default": "Utiliser la valeur par défaut",
  "Use org default": "Utiliser les valeurs par défaut de l'organisation",
  "Use org default model": "Utiliser le modèle par défaut de l'organisation",
  "Value (comma-separated for multiple)": "Valeur (séparée par des virgules pour plusieurs)",
  "View schedule {value0}": "Afficher le planning {value0}",
  "View Session": "Afficher la session",
  "View Workflow": "Afficher le flux de travail",
  "View workflow {value0}": "Afficher le flux de travail {value0}",
  "Waiting for results...": "En attente des résultats...",
  "Weight (optional, default 1.0)": "Poids (facultatif, 1.0 par défaut)",
  "What this observer watches for...": "Ce que surveille cet observateur...",
  "What this test verifies": "Ce que vérifie ce test",
  Worker: "Exécutant",
  "Worker Pool": "Pool de travailleurs",
  Workflow: "Flux de travail",
  "Workflow Details": "Détails du flux de travail",
  "Workflow input parameters": "Paramètres d'entrée du workflow",
  "Workflow Not Found": "Flux de travail introuvable",
  "workflow-type-name": "nom-type-de-workflow",
  "Yes (max {value0})": "Oui (maximum {value0})",
};
