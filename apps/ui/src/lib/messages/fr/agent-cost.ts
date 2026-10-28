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
};
