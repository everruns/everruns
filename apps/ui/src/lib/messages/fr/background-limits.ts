// French product copy for background limits (Settings → Limits, schedule limits).
export const messages: Readonly<Record<string, string>> = {
  Limits: "Limites",
  "Limit what schedules and other unattended work may run and spend":
    "Limitez ce que les planifications et autres travaux sans surveillance peuvent exécuter et dépenser",
  "Unable to load limits.": "Impossible de charger les limites.",
  "Unattended work": "Travail sans surveillance",
  "Limits for work no person started: schedules agents create, trigger runs and automatic retries. A person can still extend a schedule from its session.":
    "Limites pour le travail qu'aucune personne n'a lancé : planifications créées par les agents, exécutions de déclencheurs et nouvelles tentatives automatiques. Une personne peut toujours prolonger une planification depuis sa session.",
  "Longest an agent's schedule may run": "Durée maximale d'une planification d'agent",
  "Up to 7 days. The agent must set an end, and it cannot ask for more than this.":
    "Jusqu'à 7 jours. L'agent doit fixer une fin et ne peut pas demander davantage.",
  "Most runs per agent schedule": "Nombre maximal d'exécutions par planification d'agent",
  "Up to 96 runs. The schedule stops at this count or at its end time, whichever is first.":
    "Jusqu'à 96 exécutions. La planification s'arrête à ce nombre ou à son heure de fin, selon ce qui arrive en premier.",
  "Default budget per schedule": "Budget par défaut par planification",
  "What a schedule may spend in total when the agent does not ask for a budget.":
    "Ce qu'une planification peut dépenser au total quand l'agent ne demande pas de budget.",
  "Largest budget an agent may give a schedule":
    "Budget maximal qu'un agent peut donner à une planification",
  "The most an agent may ask for. A person can raise a single schedule past it.":
    "Le maximum qu'un agent peut demander. Une personne peut relever une planification au-delà.",
  "Daily cap on unattended work (schedules, triggers, retries)":
    "Plafond quotidien du travail sans surveillance (planifications, déclencheurs, nouvelles tentatives)",
  "What all unattended work in this organization may spend per day, together.":
    "Ce que tout le travail sans surveillance de cette organisation peut dépenser par jour, au total.",
  "No cap": "Aucun plafond",
  "Only organization owners and admins can change these limits.":
    "Seuls les propriétaires et administrateurs de l'organisation peuvent modifier ces limites.",
  "Could not save limits.": "Impossible d'enregistrer les limites.",
  runs: "exécutions",
  "Schedule lifetime must be a whole number of days from 1 to 7.":
    "La durée d'une planification doit être un nombre entier de jours entre 1 et 7.",
  "Runs per schedule must be a whole number from 1 to 96.":
    "Le nombre d'exécutions par planification doit être un nombre entier entre 1 et 96.",
  "Default budget per schedule must be more than $0.":
    "Le budget par défaut par planification doit être supérieur à 0 $.",
  "Largest schedule budget must be more than $0.":
    "Le budget maximal d'une planification doit être supérieur à 0 $.",
  "Default budget per schedule cannot be more than the largest budget.":
    "Le budget par défaut par planification ne peut pas dépasser le budget maximal.",
  "Daily cap must be more than $0, or turn on No cap.":
    "Le plafond quotidien doit être supérieur à 0 $, ou activez Aucun plafond.",
  "Unattended work (daily)": "Travail sans surveillance (quotidien)",
  "Budget used up": "Budget épuisé",
  "Stopped: repeating": "Arrêté : se répète",
  " · Run {value0} of {value1}": " · Exécution {value0} sur {value1}",
  " · Ends {value0}": " · Se termine {value0}",
  " · {value0} of {value1}": " · {value0} sur {value1}",
  "Stops when: {value0}": "S'arrête quand : {value0}",
  Extend: "Prolonger",
  "Re-enable": "Réactiver",
  "Could not re-enable the schedule.": "Impossible de réactiver la planification.",
  "Extend schedule": "Prolonger la planification",
  "Give this schedule more time, runs or budget. You can go past the limits agents are held to.":
    "Donnez à cette planification plus de temps, d'exécutions ou de budget. Vous pouvez dépasser les limites imposées aux agents.",
  "Ends at": "Se termine le",
  "Most runs": "Nombre maximal d'exécutions",
  "Runs so far: {value0}": "Exécutions jusqu'ici : {value0}",
  "Budget (USD)": "Budget (USD)",
  "Could not extend the schedule.": "Impossible de prolonger la planification.",
  "The end time must be in the future.": "L'heure de fin doit être dans le futur.",
  "Runs must be a whole number above 0.":
    "Le nombre d'exécutions doit être un nombre entier supérieur à 0.",
  "The budget must be more than $0.": "Le budget doit être supérieur à 0 $.",
  "Change at least one value to extend.": "Modifiez au moins une valeur pour prolonger.",
};
