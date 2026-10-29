export const messages: Readonly<Record<string, string>> = {
  "/ task": "/ tâche",
  "Advanced assumptions": "Hypothèses avancées",
  "Agent model": "Modèle de l’agent",
  "Allow for variation": "Prévoir des variations",
  "Answer a question or write a short reply":
    "Répondre à une question ou rédiger une réponse courte",
  "Calculating your estimate…": "Calcul de votre estimation…",
  "Check the numbers under Advanced assumptions, or choose a task to reset them.":
    "Vérifiez les nombres dans Hypothèses avancées, ou choisissez une tâche pour les réinitialiser.",
  "Choose a model for this agent before estimating its cost.":
    "Choisissez un modèle pour cet agent avant d’estimer son coût.",
  "Choose the kind of work and how often you’ll use this agent.":
    "Choisissez le type de travail et la fréquence d’utilisation de cet agent.",
  "Cost breakdown": "Détail des coûts",
  "Deep research": "Recherche approfondie",
  "Enter a whole number from 0 to 1,000,000.": "Saisissez un entier de 0 à 1 000 000.",
  "Enter a whole number from {min} to {max}.": "Saisissez un entier de {min} à {max}.",
  "Estimate cost": "Estimer le coût",
  "Estimated AI cost · USD": "Coût estimé de l’IA · USD",
  "Explore a topic and compare several sources": "Explorer un sujet et comparer plusieurs sources",
  "Find information and put together an answer": "Trouver des informations et préparer une réponse",
  "For simpler or more involved tasks": "Pour des tâches plus simples ou plus complexes",
  "Harness model": "Modèle du harnais",
  "Include follow-up calls after tools.": "Incluez les appels de suivi après les outils.",
  "Include reasoning and tool-call arguments.":
    "Incluez le raisonnement et les arguments des appels d’outils.",
  "Input · {count} tokens": "Entrée · {count} jetons",
  "Larger tasks may need a different model.":
    "Les tâches plus importantes peuvent nécessiter un autre modèle.",
  "Long-context pricing applies to one or more calls.":
    "Le tarif de contexte long s’applique à un ou plusieurs appels.",
  "Model calls per task": "Appels au modèle par tâche",
  "Monthly cost": "Coût mensuel",
  "Monthly cost assumes 30 days of use. Each task starts a new conversation.":
    "Le coût mensuel suppose 30 jours d’utilisation. Chaque tâche démarre une nouvelle conversation.",
  "Multi-step task": "Tâche en plusieurs étapes",
  "New tool results added before the next call.":
    "Nouveaux résultats d’outils ajoutés avant l’appel suivant.",
  "One task is one request, such as “research this company.”":
    "Une tâche correspond à une demande, comme « fais des recherches sur cette entreprise ».",
  "Output is capped at the configured agent or model limit in affected scenarios.":
    "La sortie est plafonnée à la limite configurée de l’agent ou du modèle dans les scénarios concernés.",
  "Output tokens per call": "Jetons de sortie par appel",
  "Output · {count} tokens": "Sortie · {count} jetons",
  "Quick answer": "Réponse rapide",
  Retry: "Réessayer",
  "Starting input tokens": "Jetons d’entrée initiaux",
  "Task assumptions": "Hypothèses de la tâche",
  "Tasks per day": "Tâches par jour",
  "The task choices fill in these assumptions for you. Adjust them if you know how much work your agent usually does.":
    "Les types de tâches remplissent ces hypothèses. Ajustez-les si vous connaissez le travail habituel de votre agent.",
  "This task is too large for the selected model. Try a smaller task or adjust Advanced assumptions.":
    "Cette tâche est trop grande pour le modèle choisi. Essayez une tâche plus petite ou ajustez les Hypothèses avancées.",
  "Tool result tokens per call": "Jetons de résultats d’outils par appel",
  "Using {model}": "Avec {model}",
  "We can’t estimate this setup yet. Check the model and the agent’s output limit.":
    "Nous ne pouvons pas encore estimer cette configuration. Vérifiez le modèle et la limite de sortie de l’agent.",
  "We couldn’t load your estimate. Please try again.":
    "Impossible de charger votre estimation. Réessayez.",
  "We don’t have prices for this model yet. Try another model to see an estimate.":
    "Nous n’avons pas encore les prix de ce modèle. Essayez un autre modèle pour obtenir une estimation.",
  "What does a task look like?": "À quoi ressemble une tâche ?",
  "What will this agent cost?": "Combien coûtera cet agent ?",
  "Where the cost comes from": "D’où vient le coût",
  "Your message and any starting context.": "Votre message et le contexte initial.",
  "{count} tasks per month": "{count} tâches par mois",
  "{range} for lighter to heavier tasks": "{range} pour des tâches plus simples à plus complexes",
  "Enter tasks per day below": "Saisissez les tâches par jour ci-dessous",
  "A guide, not a fixed price or spending limit. Includes AI usage only. Paid tools, hosting and other service fees are extra. Trying this calculator is free and won’t run or change your agent.":
    "Un repère, pas un prix fixe ni un plafond de dépenses. Seule l’utilisation de l’IA est incluse. Outils payants, hébergement et autres services sont en supplément. Ce calculateur est gratuit et ne lance ni ne modifie votre agent.",
  "Includes roughly {count} tokens of instructions, tools and message overhead on every call. Previous outputs and tool results accumulate in the conversation.":
    "Chaque appel inclut environ {count} jetons d’instructions, d’outils et de structure des messages. Les sorties précédentes et résultats d’outils s’accumulent dans la conversation.",
  "Planning estimate, not a quote or spending limit. Lighter and heavier scenarios use half to twice the model calls, output and tool results. This is a sensitivity range, not a confidence interval or a maximum.":
    "Estimation de planification, pas un devis ni un plafond. Les scénarios plus légers et plus lourds utilisent de la moitié au double des appels, de la sortie et des résultats d’outils. C’est une plage de sensibilité, pas un intervalle de confiance ni un maximum.",
  "Uses catalog token prices and approximate text sizing, with no cache discount. Actual costs depend on the task, retries, reasoning, cache reads/writes and provider pricing. Excludes sandbox time, paid tools, media, extra models or subagents, and platform fees.":
    "Utilise les prix du catalogue et une taille de texte approximative, sans réduction de cache. Le coût réel dépend de la tâche, des tentatives, du raisonnement, des lectures/écritures du cache et du fournisseur. Exclut sandbox, outils payants, médias, modèles supplémentaires, sous-agents et frais de plateforme.",
  "Uses the current agent draft. Estimator inputs do not change the agent. Run representative tasks in Playground and compare their recorded usage before setting a budget.":
    "Utilise le brouillon actuel de l’agent. Les saisies ne modifient pas l’agent. Exécutez des tâches représentatives dans Playground et comparez leur utilisation enregistrée avant de fixer un budget.",
  "Smart estimate": "Estimation intelligente",
  "Manual calculator": "Calculateur manuel",
  "What should we plan for? (optional)": "Que faut-il prévoir ? (facultatif)",
  "Infer typical tasks from this agent": "Déduire les tâches habituelles de cet agent",
  "Update estimate": "Actualiser l’estimation",
  "AI suggests tasks from your agent. Usage sliders update instantly without another AI call.":
    "L’IA suggère des tâches selon votre agent. Les curseurs actualisent la projection sans nouvel appel à l’IA.",
  "Understanding your agent and estimating typical tasks…":
    "Analyse de votre agent et estimation des tâches habituelles…",
  "Smart estimates are unavailable right now. Retry or use the manual calculator.":
    "L’estimation intelligente est indisponible. Réessayez ou utilisez le calculateur manuel.",
  "Cost projection": "Projection des coûts",
  "Average per task": "Moyenne par tâche",
  "Daily cost": "Coût quotidien",
  "30-day month · Each task starts a new conversation":
    "Mois de 30 jours · Chaque tâche ouvre une nouvelle conversation",
  "Check task assumptions and select at least one task. Oversized tasks cannot be priced.":
    "Vérifiez les hypothèses et sélectionnez au moins une tâche. Les tâches trop volumineuses ne peuvent pas être chiffrées.",
  "Usage projection": "Projection d’utilisation",
  "By tasks": "Par tâches",
  "By users": "Par utilisateurs",
  "Active users": "Utilisateurs actifs",
  "Tasks per user per day": "Tâches par utilisateur et par jour",
  "Enter a whole number within the displayed limits.":
    "Saisissez un entier dans les limites indiquées.",
  "Task mix": "Répartition des tâches",
  "Typical tasks for your agent": "Tâches habituelles de votre agent",
  "Suggested by AI, not measured usage. Equal mix to start; adjust relative frequency or set a task to zero.":
    "Suggestions de l’IA, pas des mesures. Répartition égale au départ ; ajustez la fréquence relative ou mettez une tâche à zéro.",
  Frequency: "Fréquence",
  "Monthly spend as usage grows": "Coût mensuel selon l’utilisation",
  "Monthly planning range: {low} – {high}": "Fourchette mensuelle : {low} – {high}",
  "Largest cost driver: {task}": "Principal poste de coût : {task}",
  "Edit task": "Modifier la tâche",
  "Task example": "Exemple de tâche",
  "Changing the example does not recalculate assumptions. Use the planning input above to generate a new estimate.":
    "Modifier l’exemple ne recalcule pas les hypothèses. Utilisez le champ ci-dessus pour une nouvelle estimation.",
  "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Current catalog prices and model limits apply. Cache discounts are not assumed.":
    "Les instructions et outils sont comptés à chaque appel ; les réponses et résultats précédents s’accumulent. Les tarifs du catalogue et limites du modèle s’appliquent, sans remise de cache présumée.",
  "Planning scenarios, not quotes or spending limits. The shaded range varies task complexity, not statistical confidence. AI token costs only: paid tools, hosting, media, other agents and service fees are extra. No tasks are executed and your agent is not changed.":
    "Scénarios de planification, pas des devis ni des plafonds. La zone ombrée varie la complexité, sans confiance statistique. Coût des tokens IA uniquement : outils payants, hébergement, médias, autres agents et frais sont en supplément. Aucune tâche exécutée, aucun agent modifié.",
  "{label} slider": "Curseur {label}",
  "Monthly cost projection with lighter and heavier task scenarios":
    "Projection mensuelle pour des tâches simples ou complexes",
  "{count} tasks / day": "{count} tâches / jour",
  "At current usage": "À l’utilisation actuelle",
  "/ month": "/ mois",
  "Sandbox runtime": "Durée du bac à sable",
  "Paid tools": "Outils payants",
  "Media generation": "Génération de médias",
  "Subagent work": "Travail des sous-agents",
  "Hosting and platform fees": "Hébergement et frais de plateforme",
  "Other operating costs": "Autres coûts de fonctionnement",
  "Cost category": "Catégorie de coût",
  "Add cost": "Ajouter un coût",
  "Paid from credits": "Prélevé sur les crédits",
  "Billed externally": "Facturé séparément",
  "Quantity per month": "Quantité par mois",
  "Quantity per task": "Quantité par tâche",
  "USD per {unit}": "USD par {unit}",
  "Enter provider price": "Saisir le tarif du fournisseur",
  "Estimated operating cost · USD": "Coût de fonctionnement estimé · USD",
  "External bills": "Factures externes",
  "Loading service prices…": "Chargement des tarifs…",
  "Service prices are unavailable. Reopen the estimator to retry.":
    "Les tarifs sont indisponibles. Rouvrez l’estimateur pour réessayer.",
  "Prices needed for: {items}": "Tarifs nécessaires pour : {items}",
  "Check resource quantities and prices. Use nonnegative numbers.":
    "Vérifiez les quantités et les tarifs. Utilisez des nombres positifs ou nuls.",
  "Largest model cost driver: {task}": "Principal coût du modèle : {task}",
  "Extra model spend for retries (%)": "Surcoût du modèle pour les nouvelles tentatives (%)",
  "Cached input reads (%)": "Lecture d’entrée en cache (%)",
  "Cache writes (%)": "Écriture en cache (%)",
  "Monthly estimate: {amount}": "Estimation mensuelle : {amount}",
  "Model usage": "Utilisation du modèle",
  minute: "minute",
  call: "appel",
  item: "élément",
  task: "tâche",
  month: "mois",
  cpu_second: "seconde CPU",
  search: "recherche",
  request: "requête",
  MB: "Mo",
  "Include resources used by an average task. External providers bill separately from prepaid credits.":
    "Ajoutez les ressources utilisées par une tâche moyenne. Les fournisseurs externes facturent séparément des crédits prépayés.",
  "Use the full sandbox instance price, including memory. For subagents, enter the cost of one child task. Include idle time and repeated tool work in quantities; monthly fees are added once.":
    "Utilisez le prix complet du bac à sable, mémoire comprise. Pour les sous-agents, indiquez le coût d’une tâche enfant. Incluez le temps d’inactivité et les répétitions ; les frais mensuels sont ajoutés une seule fois.",
  "Per-task cost excludes fixed monthly fees. Daily cost includes one thirtieth of monthly fees. Only included resources are estimated.":
    "Le coût par tâche exclut les frais mensuels fixes. Le coût journalier comprend un trentième des frais mensuels. Seules les ressources ajoutées sont estimées.",
  "Cache shares apply to input tokens across all tasks and must total at most 100%. Retry allowance increases model spend only; repeated tools and sandbox time belong in resource quantities.":
    "Les parts de cache concernent les tokens d’entrée de toutes les tâches et leur total ne doit pas dépasser 100 %. La marge de nouvelle tentative augmente uniquement le coût du modèle ; ajoutez les outils répétés et le temps de bac à sable aux quantités.",
  "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Catalog prices and model limits apply. Cache savings require explicit assumptions.":
    "Les instructions et les outils sont comptés à chaque appel ; les réponses et résultats précédents s’accumulent. Les tarifs du catalogue et limites du modèle s’appliquent. Les économies de cache nécessitent des hypothèses explicites.",
  "Planning scenarios, not quotes or spending limits. The shaded range varies model workload; resource assumptions stay fixed. Add paid tools, sandbox time, media, subagents and fees to include them. No tasks are executed and your agent is not changed.":
    "Scénarios de planification, pas des devis ni des plafonds. La zone ombrée fait varier le travail du modèle ; les ressources restent fixes. Ajoutez les outils payants, bacs à sable, médias, sous-agents et frais pour les inclure. Aucune tâche n’est exécutée et l’agent n’est pas modifié.",
};
