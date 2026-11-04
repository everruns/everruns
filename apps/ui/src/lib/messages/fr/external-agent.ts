export const messages: Readonly<Record<string, string>> = {
  "External agent": "Agent externe",
  "Run this agent's turns on your own HTTPS endpoint.":
    "Exécutez les tours de cet agent sur votre propre point de terminaison HTTPS.",
  "Your endpoint runs this agent's turns.":
    "Votre point de terminaison exécute les tours de cet agent.",
  "Your code runs each turn with its own system prompt, model and tools. Sessions, Playground, Trace and channels keep working. While an endpoint is connected, the page hides the settings it owns.":
    "Votre code exécute chaque tour avec son propre prompt système, son modèle et ses outils. Les sessions, Playground, Trace et les canaux continuent de fonctionner. Tant qu'un point de terminaison est connecté, la page masque les paramètres qu'il gère.",
  "Your endpoint runs every turn with its own system prompt, model and tools. Everruns keeps the sessions, channels, traces and usage for this agent.":
    "Votre point de terminaison exécute chaque tour avec son propre prompt système, son modèle et ses outils. Everruns conserve les sessions, canaux, traces et l'utilisation de cet agent.",
  "Harness, capabilities, model, MCP servers, credentials, files, network access, sandbox and token limits are hidden: your endpoint owns them.":
    "Le harness, les capacités, le modèle, les serveurs MCP, les identifiants, les fichiers, l'accès réseau, le sandbox et les limites de jetons sont masqués : votre point de terminaison les gère.",
  "Deploy your endpoint": "Déployez votre point de terminaison",
  "Any HTTPS service that implements the endpoint protocol.":
    "Tout service HTTPS qui implémente le protocole du point de terminaison.",
  "Cloudflare agent guide": "Guide de l'agent Cloudflare",
  "Install an endpoint secret": "Installez un secret de point de terminaison",
  "Set it in your endpoint's environment before you connect, for example with wrangler secret put EVERRUNS_ENDPOINT_SECRET. At least 32 characters. Everruns stores it encrypted and does not show it again.":
    "Définissez-le dans l'environnement du point de terminaison avant de vous connecter, par exemple avec wrangler secret put EVERRUNS_ENDPOINT_SECRET. Au moins 32 caractères. Everruns le stocke chiffré et ne l'affiche plus.",
  "Endpoint URL": "URL du point de terminaison",
  "Everruns sends a signed challenge to check that the endpoint holds the secret. After you connect, messages and session history for this agent go to the endpoint. Running turns must finish first.":
    "Everruns envoie un défi signé pour vérifier que le point de terminaison détient le secret. Une fois connecté, les messages et l'historique des sessions de cet agent sont envoyés au point de terminaison. Les tours en cours doivent d'abord se terminer.",
  "Verify and connect": "Vérifier et connecter",
  "Could not connect the endpoint.": "Impossible de connecter le point de terminaison.",
  "Could not disconnect the endpoint.": "Impossible de déconnecter le point de terminaison.",
  Disconnected: "Déconnecté",
  Queue: "File d'attente",
  "New messages wait until an endpoint is connected again.":
    "Les nouveaux messages attendent qu'un point de terminaison soit reconnecté.",
  "The last call failed": "Le dernier appel a échoué",
  "Open the session's trace for details.": "Consultez la trace de la session pour plus de détails.",
  "Change the endpoint or rotate the secret":
    "Changer le point de terminaison ou renouveler le secret",
  "Install the new secret in your endpoint first, then verify again. The current connection keeps working until the new one is verified.":
    "Installez d'abord le nouveau secret dans votre point de terminaison, puis vérifiez à nouveau. La connexion actuelle continue de fonctionner jusqu'à la vérification de la nouvelle.",
  "Update connection": "Mettre à jour la connexion",
  "Disconnect endpoint": "Déconnecter le point de terminaison",
  "New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.":
    "Les nouveaux messages attendent dans la file jusqu'à ce qu'un point de terminaison soit reconnecté. Everruns ne les exécute pas lui-même.",
  "Disconnect the endpoint? New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.":
    "Déconnecter le point de terminaison ? Les nouveaux messages attendent dans la file jusqu'à ce qu'un point de terminaison soit reconnecté. Everruns ne les exécute pas lui-même.",
  "Connecting an endpoint requires permission to change dangerous agent settings.":
    "La connexion d'un point de terminaison nécessite l'autorisation de modifier les paramètres sensibles de l'agent.",
};
