export const messages: Readonly<Record<string, string>> = {
  "Compaction quality monitoring": "Suivi de la qualité de la compaction",
  "Automatically check compaction in real sessions. Operational checks use no judge model; sampled continuity checks use the organization model selected below.":
    "Vérifiez automatiquement la compaction dans les sessions réelles. Les contrôles opérationnels n’utilisent pas de modèle juge ; les contrôles de continuité échantillonnés utilisent le modèle de l’organisation choisi ci-dessous.",
  "Findings stay in this organization. Sampled excerpts are processed by its selected provider. Monitoring is read-only, and findings are retained for 30 days.":
    "Les résultats restent dans cette organisation. Les extraits échantillonnés sont traités par son fournisseur sélectionné. Le suivi est en lecture seule et les résultats sont conservés 30 jours.",
  "Enable automatic monitoring": "Activer le suivi automatique",
  "Monitoring model": "Modèle de suivi",
  "Mechanical checks only": "Contrôles mécaniques uniquement",
  "Daily monitoring budget (USD)": "Budget quotidien de suivi (USD)",
  "Continuity sample (%)": "Échantillon de continuité (%)",
  "Excluded session tags": "Étiquettes de session exclues",
  "A zero budget enables mechanical checks only. The daily UTC cap includes conservative retry reservations; uncertain calls keep their allowance for that day. Disabling monitoring stops new assessments, including queued work.":
    "Un budget nul active uniquement les contrôles mécaniques. Le plafond quotidien UTC comprend des réservations prudentes pour les nouvelles tentatives ; les appels incertains gardent leur réservation pour la journée. La désactivation arrête les nouvelles évaluations, y compris celles en attente.",
  "Monitoring coverage · 7 days": "Couverture du suivi · 7 jours",
  "Could not load monitoring coverage.": "Impossible de charger la couverture du suivi.",
  "Today: {reserved} reserved allowance · {reported} provider-reported cost":
    "Aujourd’hui : {reserved} réservés · {reported} de coût déclaré par le fournisseur",
  "No compaction assessments yet.": "Aucune évaluation de compaction pour le moment.",
  "Continuity checks": "Contrôles de continuité",
  "Operational checks": "Contrôles opérationnels",
  "Passed {passed} · Failed {failed} · Insufficient evidence {unknown} · Pending {pending}":
    "Réussites {passed} · Échecs {failed} · Preuves insuffisantes {unknown} · En attente {pending}",
  "Recent findings": "Résultats récents",
  "An operational pass confirms the reported execution step. Continuity verdicts apply to sampled evidence and do not prove complete memory preservation or causality.":
    "Une réussite opérationnelle confirme l’étape d’exécution signalée. Les verdicts de continuité portent sur des preuves échantillonnées et ne prouvent ni une conservation complète de la mémoire ni un lien de causalité.",
  "Could not load monitoring findings.": "Impossible de charger les résultats du suivi.",
  "Insufficient evidence": "Preuves insuffisantes",
  "Select a monitoring model before enabling paid checks.":
    "Choisissez un modèle de suivi avant d’activer les contrôles payants.",
  "Compaction monitoring": "Suivi de la compaction",
  "System-managed checks configured in organization settings.":
    "Contrôles gérés par le système et configurés dans les paramètres de l’organisation.",
  "Agree with assessment": "Approuver l’évaluation",
  "Disagree with assessment": "Contester l’évaluation",
  "Review recorded": "Avis enregistré",
  "Could not save assessment review.": "Impossible d’enregistrer l’avis sur l’évaluation.",
};
