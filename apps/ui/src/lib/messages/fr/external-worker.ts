export const messages: Readonly<Record<string, string>> = {
  "External worker setup failed": "Échec de la configuration du worker externe",
  Execution: "Exécution",
  "External worker": "Worker externe",
  "Built-in runtime": "Environnement intégré",
  "Configure execution": "Configurer l’exécution",
  "Unable to load execution settings": "Impossible de charger les paramètres d’exécution",
  "This endpoint owns the model, prompt and tools. Agent editor settings are not applied to external execution.":
    "Ce point de terminaison contrôle le modèle, les instructions et les outils. Les paramètres de l’éditeur d’agent ne s’appliquent pas à l’exécution externe.",
  "Deploy your worker first. Install a random endpoint secret in its environment, then enter it here to verify ownership. Messages and session history for this Agent are sent to the endpoint.":
    "Déployez d’abord votre worker. Installez un secret aléatoire dans son environnement, puis saisissez-le ici pour vérifier que vous le contrôlez. Les messages et l’historique des sessions de cet agent sont envoyés au point de terminaison.",
  "Worker HTTPS endpoint": "Point de terminaison HTTPS du worker",
  "Endpoint secret": "Secret du point de terminaison",
  "Verify and connect": "Vérifier et connecter",
  "Disable external worker": "Désactiver le worker externe",
};
