// Product-owned copy. Preserve interpolation tokens and stable identifiers.
export const messages: Readonly<Record<string, string>> = {
  Advanced: "Avancé",
  Spending: "Dépenses",
  "The shared pool covers everyone on this channel. The daily limit caps each person until midnight UTC.":
    "Le pool partagé couvre toutes les personnes de ce canal. La limite quotidienne plafonne chaque personne jusqu'à minuit UTC.",
  "Caps what this channel can spend, separately from the agent's budget.":
    "Plafonne ce que ce canal peut dépenser, séparément du budget de l'agent.",
  "Shared pool": "Pool partagé",
  "Each person, per day (USD)": "Chaque personne, par jour (USD)",
  "No daily cap": "Pas de plafond quotidien",
  "A person who reaches this cannot start new work until the next UTC day.":
    "Une personne qui atteint ce montant ne peut plus lancer de nouveau travail avant le jour UTC suivant.",
  "Who can use this chat": "Qui peut utiliser ce chat",
  "Open access": "Accès ouvert",
  "Anyone can register and sign in.": "Toute personne peut créer un compte et se connecter.",
  "Single user + invites": "Une personne et des invitations",
  "Only one person, plus the people they invite.":
    "Une seule personne, plus les personnes qu'elle invite.",
  "Organization users": "Utilisateurs de l'organisation",
  "Members of this organization can sign in.":
    "Les membres de cette organisation peuvent se connecter.",
  "This person": "Cette personne",
  "They sign in with this email.": "Cette personne se connecte avec cette adresse e-mail.",
  Invites: "Invitations",
  "One email per line. Each person signs in with their own account.":
    "Une adresse par ligne. Chaque personne se connecte avec son propre compte.",
  "Enter this person's email.": "Saisissez l'adresse e-mail de cette personne.",
  "Use email addresses, one per line, up to 50.":
    "Indiquez des adresses e-mail, une par ligne, jusqu'à 50.",
  "Require visitors to sign in with Google. When on, every request must present a valid Google account; signed-in visitors skip the Turnstile challenge.":
    "Demander aux visiteurs de se connecter avec Google. Lorsque cette option est activée, chaque demande doit présenter un compte Google valide ; les visiteurs connectés ignorent le défi Turnstile.",
  Required: "Obligatoire",
  "Required by:": "Requis par :",
  Requires: "Nécessite",
  "Requires Contents write permission in the GitHub App and installation. Saved edits are committed directly to the selected branch. Branch protection still applies. File paths are preserved; manifest formatting may be normalized.":
    "Nécessite une autorisation d'écriture du contenu dans l'application GitHub et l'installation. Les modifications enregistrées sont directement validées dans la branche sélectionnée. La protection des succursales s’applique toujours. Les chemins de fichiers sont conservés ; le formatage du manifeste peut être normalisé.",
  "Response timeout (seconds)": "Délai d'expiration de réponse (secondes)",
  "Result limit": "Limite de résultat",
  "Retrieve relevant context automatically": "Récupérer automatiquement le contexte pertinent",
  "Returned verbatim for GET on the channel URL. Use plain Markdown.":
    "Renvoyé textuellement pour GET sur l'URL de la chaîne. Utilisez du Markdown simple.",
  "Review GitHub version": "Vérifier la version de GitHub",
  "Revoke key": "Révoquer la clé",
  "Revoking...": "Révoquer...",
  "Rotate API key": "Rotation de la clé API",
  "Rotating...": "Rotation...",
  "Run {{app.name}} now.": "Exécutez {{app.name}} maintenant.",
  "Run history will appear here when channel run aggregation is available.":
    "L'historique des exécutions apparaîtra ici lorsque l'agrégation des exécutions de canaux est disponible.",
  "Run now": "Exécuter maintenant",
  "Run the daily digest": "Exécutez le résumé quotidien",
  "Running…": "En cours d'exécution…",
  "Runs · 24h": "Fonctionne · 24h",
  "Runs generated smoke tests as real sessions, then scores them with an AI judge. Takes a minute or two.":
    "Exécute les tests de fumée générés comme des sessions réelles, puis les note avec un juge IA. Cela prend une minute ou deux.",
  "Runs once when Everruns creates replacement compute. Do not put credentials here.":
    "S'exécute une fois lorsque Everruns crée un calcul de remplacement. Ne mettez pas d'informations d'identification ici.",
  "Runs, 24h": "Fonctionnement, 24h",
  "Runtime default": "Valeur par défaut d'exécution",
  "Sandbox {value0}": "Bac à sable {value0}",
  "Sandbox binding name {value0}": "Nom de liaison du bac à sable {value0}",
  "Save avatar": "Enregistrer l'avatar",
  "Save changes before opening Slack.": "Enregistrez les modifications avant d'ouvrir Slack.",
  "Save channel": "Enregistrer la chaîne",
  "Save configuration changes before publishing.":
    "Enregistrez les modifications de configuration avant la publication.",
  "Save settings first to generate an updated manifest. Update your existing app in Slack and reinstall it when permissions change.":
    "Enregistrez d'abord les paramètres pour générer un manifeste mis à jour. Mettez à jour votre application existante dans Slack et réinstallez-la lorsque les autorisations changent.",
  "Save the channel to add it to Slack in one click. If you already have a Slack app, open Configure manually below.":
    "Enregistrez la chaîne pour l'ajouter à Slack en un clic. Si vous disposez déjà d’une application Slack, ouvrez Configurer manuellement ci-dessous.",
  "Saving avatar…": "Sauvegarde de l'avatar…",
  "Schedule ·": "Horaire ·",
  "Schedule preview": "Aperçu du calendrier",
  "Scheduled for removal. It still works today; plan to replace it before it is removed.":
    "Suppression prévue. Cela fonctionne encore aujourd'hui ; prévoyez de le remplacer avant de le retirer.",
  "Search account (not connected)": "Rechercher un compte (non connecté)",
  "Search avatars": "Rechercher des avatars",
  "Search by name or description": "Recherche par nom ou description",
  "Search capabilities": "Capacités de recherche",
  "Search capabilities...": "Capacités de recherche...",
  "Search channels…": "Rechercher des chaînes…",
  "Search names, roles, animals, colors…":
    "Rechercher des noms, des rôles, des animaux, des couleurs…",
  "Search presets": "Rechercher des préréglages",
  "Search provider": "Fournisseur de recherche",
  "Select a harness": "Sélectionnez un cadre d’exécution",
  "Select a repository": "Sélectionnez un référentiel",
  "Select a repository, branch, and package folder or manifest. Review changes before enabling continuous sync. Enabling applies the reviewed GitHub version and replaces local authored changes. Choose whether edits can also be saved from Everruns to GitHub.":
    "Sélectionnez un référentiel, une branche et un dossier ou un manifeste de package. Examinez les modifications avant d’activer la synchronisation continue. L'activation applique la version révisée de GitHub et remplace les modifications créées localement. Choisissez si les modifications peuvent également être enregistrées depuis Everruns vers GitHub.",
  "Select an account": "Sélectionnez un compte",
  "Select Capabilities": "Sélectionnez les capacités",
  "Select the indexes this capability can search.":
    "Sélectionnez les index que cette fonctionnalité peut rechercher.",
  "Selectable: declared sandboxes only": "Sélectionnable : bacs à sable déclarés uniquement",
  Selected: "Sélectionné",
  "Selected avatar in square and circle": "Avatar sélectionné en carré et cercle",
  "Selected collections mount read-only. Unindexed files remain available to read and grep.":
    "Les collections sélectionnées sont montées en lecture seule. Les fichiers non indexés restent disponibles en lecture et en saisie.",
  "Selected repositories": "Dépôts sélectionnés",
  "Selected window": "Fenêtre sélectionnée",
  "Send as Authorization: Bearer <token> or X-Everruns-Webhook-Token.":
    "Envoyer comme autorisation : Bearer <token> ou X-Everruns-Webhook-Token.",
  "Sends an excerpt to the utility model.": "Envoie un extrait au modèle d'utilité.",
  server: "serveur",
  "Servers added to this agent, plus any its capabilities and harness bring.":
    "Serveurs ajoutés à cet agent, ainsi que toutes ses capacités et son cadre d'exécution.",
  "Service account": "Compte de service",
  "Service identity": "Identité du service",
  "Session choice": "Choix de session",
  "Session expiration (hours)": "Expiration de la session (heures)",
  "Session mode": "Mode session",
  "Session Mode": "Mode session",
  sessions: "sessions",
  "Sessions via channels": "sessions via canaux",
  "Set PUBLIC_APP_URL to a public HTTPS origin, restart Everruns, and reload before creating the Slack app.":
    "Définissez PUBLIC_APP_URL sur une origine HTTPS publique, redémarrez Everruns et rechargez avant de créer l'application Slack.",
  "Set the avatar": "Définir l'avatar",
  "Set up": "Configurer",
  "Set up {value0}": "Configurer {value0}",
  "Set up agent": "Agent de configuration",
  settings: "paramètres",
  "Settings → Slack workspaces": "Paramètres → Espaces de travail Slack",
  "Shared session": "Session partagée",
  "Shared token (optional)": "Jeton partagé (facultatif)",
  "shared-secret": "secret partagé",
  "Shown as an intro box on a fresh thread. Images are allowed. Hidden once the user types.":
    "Affiché comme boîte d'introduction sur un nouveau fil de discussion. Les images sont autorisées. Masqué une fois que l'utilisateur tape.",
  "Shown before the visitor's first message.": "Affiché avant le premier message du visiteur.",
  "Shown in logs": "Affiché dans les journaux",
  "Sign-in required": "Connexion requise",
  "Signing secret": "Secret de signature",
  "Single chat with threads": "Chat unique avec fils de discussion",
  "Single chat with threads requires the Agent’s coordinator capability and thread tools.":
    "Une conversation unique avec des threads nécessite la capacité de coordinateur et les outils de thread de l'agent.",
  "Skills:": "Compétences :",
  "Skipped user servers": "Serveurs d'utilisateurs ignorés",
  Slack: "Slack",
  "Slack app manifest": "Manifeste de l'application Slack",
  "Slack channel configuration": "Configuration du canal Slack",
  "Slack connection": "Connexion lâche",
  "Slack install did not start": "L'installation de Slack n'a pas démarré",
  "Slack signing secret": "Secret de signature Slack",
  "Slack stopped accepting the saved credential. Reconnect once for the organization, then add agents with one click.":
    "Slack n'accepte plus les informations d'identification enregistrées. Reconnectez-vous une fois pour l'organisation, puis ajoutez des agents en un seul clic.",
  "Slack verified the Request URL. Invite the bot to a channel, then @mention it to test.":
    "Slack a vérifié l'URL de la demande. Invitez le bot sur une chaîne, puis @mentionnez-le pour tester.",
  "Slack workspace": "Espace de travail Slack",
  "Slack workspace:": "Espace de travail Slack :",
  "Source:": "Source :",
  "Speaking style": "Style de parole",
  "Speech always stops. Keep working lets the caller's next words steer the answer in progress; stop and start over cancels it.":
    "La parole s'arrête toujours. Continuez à travailler, laissez les prochains mots de l'appelant orienter la réponse en cours ; arrêter et recommencer l'annule.",
  "Speech model": "Modèle vocal",
  "Spoken when the call connects. Tell callers they are talking to an AI.":
    "Parlé lorsque l'appel est connecté. Dites aux appelants qu’ils parlent à une IA.",
  "Spoken while the agent works and has said nothing yet. 0 turns fillers off; max 60000.":
    "Parlé pendant que l'agent travaille et n'a encore rien dit. 0 désactive les remplissages ; maximum 60 000.",
  Stage: "Étape",
  "Start from an example": "Partir d'un exemple",
  "Start over": "Recommencer",
  "Start the sandbox with a Linux desktop and give the agent the computer tool to see and use it. Screenshots cost image tokens.":
    "Démarrez le bac à sable avec un bureau Linux et donnez à l'agent l'outil informatique pour le voir et l'utiliser. Les captures d'écran coûtent des jetons d'image.",
  "Starting files for new sessions. Updating these files does not change existing sessions.":
    "Démarrage des fichiers pour les nouvelles sessions. La mise à jour de ces fichiers ne modifie pas les sessions existantes.",
  "Starts on GitHub": "Démarre sur GitHub",
  Stop: "Arrêter",
  "Stop and start over": "Arrêtez et recommencez",
  "Stop: use only this provider": "Stop : utilisez uniquement ce fournisseur",
  "Suggested fix": "Correctif suggéré",
  "Support backend": "Prise en charge du back-end",
  "Sync direction": "Sens de synchronisation",
  "Sync from GitHub · read-only in Everruns":
    "Synchronisation depuis GitHub · lecture seule dans Everruns",
  "Sync with GitHub": "Synchroniser avec GitHub",
  "Sync with GitHub · edit on either side": "Synchroniser avec GitHub · modifier de chaque côté",
  T0123456789: "T0123456789",
  "Talk again": "Parlez à nouveau",
  "Talk to this channel": "Parlez à cette chaîne",
  "Tell the model which provider answered": "Dites au modèle quel fournisseur a répondu",
  "Templates can reference": "Les modèles peuvent faire référence",
  "The agent gets its own GitHub App, installed on the repositories you choose. It reads code and posts as its own bot. The App can only be installed where it is created.":
    "L'agent obtient sa propre application GitHub, installée sur les référentiels de votre choix. Il lit le code et publie comme son propre robot. L'application ne peut être installée que là où elle a été créée.",
  "The agent sees one line for this server and loads its tools through tool search when it needs them.":
    "L'agent voit une ligne pour ce serveur et charge ses outils via la recherche d'outils lorsqu'il en a besoin.",
  "The agent stops acting as the Everruns App. The installation stays linked to the organization. GitHub triggers will stop firing.":
    "L'agent cesse d'agir en tant qu'application Everruns. L'installation reste liée à l'organisation. Les déclencheurs GitHub cesseront de se déclencher.",
  "The agent's GitHub App exists but is not installed yet. Finish installing it and pick the repositories the agent may use.":
    "L'application GitHub de l'agent existe mais n'est pas encore installée. Terminez l'installation et choisissez les référentiels que l'agent peut utiliser.",
  "The channel was saved.": "La chaîne a été enregistrée.",
  "The client ID of your app in the AgentID console. Tokens must be issued for it and last ten minutes; AgentID is a sign-in, not a long-lived API key.":
    "L'ID client de votre application dans la console AgentID. Des jetons doivent être émis pour cela et durer dix minutes ; AgentID est une connexion, pas une clé API de longue durée.",
  "The complete system prompt including harness and capability additions":
    "L'invite complète du système, y compris le cadre d'exécution et les ajouts de capacités",
  "The job": "Le travail",
  "The last successfully synced definition is retained.":
    "La dernière définition synchronisée avec succès est conservée.",
  "The name this agent uses when it reads code and comments on pull requests":
    "Le nom que cet agent utilise lorsqu'il lit le code et commente les demandes d'extraction",
  "The organization also blocks": "L'organisation bloque également",
  "The plaintext key is shown once at create / rotate time. Only the prefix is stored for display; the full value is hashed at rest.":
    "La clé en texte brut est affichée une fois au moment de la création/rotation. Seul le préfixe est stocké pour l'affichage ; la valeur totale est hachée au repos.",
  "The saved agent changed while you were editing. Your unsaved draft is kept. Discard it to review the latest definition before saving.":
    "L'agent enregistré a changé pendant la modification. Votre brouillon non enregistré est conservé. Jetez-le pour consulter la dernière définition avant de l’enregistrer.",
  "The secret key is write-only and never shown after saving.":
    "La clé secrète est en écriture seule et n'est jamais affichée après l'enregistrement.",
  "The selection is smaller than 64×64 pixels. Zoom out.":
    "La sélection est inférieure à 64×64 pixels. Effectuez un zoom arrière.",
  "The server's tools are listed at the start of every turn.":
    "Les outils du serveur sont répertoriés au début de chaque tour.",
  "The utility model returns allow or block.": "Le modèle d'utilité renvoie autoriser ou bloquer.",
  "These fields are optional when saving. Slack requests are rejected until a signing secret is set; a bot token is needed to send replies.":
    "Ces champs sont facultatifs lors de l'enregistrement. Les demandes Slack sont rejetées jusqu'à ce qu'un secret de signature soit défini ; un jeton de bot est nécessaire pour envoyer des réponses.",
  "This agent has its own Slack app in workspace":
    "Cet agent possède sa propre application Slack dans l'espace de travail",
  "This agent is not reachable from outside.": "Cet agent n'est pas joignable de l'extérieur.",
  "This agent is read-only.": "Cet agent est en lecture seule.",
  "This agent may have been deleted, moved to another organization, or the URL may be wrong.":
    "Cet agent a peut-être été supprimé, déplacé vers une autre organisation ou l'URL est peut-être erronée.",
  "This agent's configuration changed since this run. Re-run to refresh results.":
    "La configuration de cet agent a changé depuis cette exécution. Réexécutez pour actualiser les résultats.",
  "This agent's GitHub App was created before CI triggers and lacks":
    "L'application GitHub de cet agent a été créée avant le déclenchement de CI et manque",
  "This browser cannot place voice calls.": "Ce navigateur ne peut pas passer d'appels vocaux.",
  "This channel fires on its own, so there is no inbound URL to call.":
    "Ce canal se déclenche tout seul, il n'y a donc aucune URL entrante à appeler.",
  "This channel may have been deleted, moved to another agent, or the URL may be wrong.":
    "Ce canal a peut-être été supprimé, déplacé vers un autre agent ou l'URL est peut-être erronée.",
  "This channel's configuration is managed by GitHub. Edit the Agent source repository to change it. Publication and credentials remain available from Integrations.":
    "La configuration de ce canal est gérée par GitHub. Modifiez le référentiel source de l'agent pour le modifier. La publication et les informations d'identification restent disponibles auprès des intégrations.",
  "This check uses a shape the editor does not recognize. Removing it is the only edit that keeps the rest of the config.":
    "Cette vérification utilise une forme que l'éditeur ne reconnaît pas. La supprimer est la seule modification qui conserve le reste de la configuration.",
  "This deployment ships no examples.": "Ce déploiement ne fournit aucun exemple.",
  "This is the only time it is shown. Store it in your application's secrets now.":
    "C'est la seule fois où il est affiché. Stockez-le dès maintenant dans les secrets de votre application.",
  "This organization has no": "Cette organisation n'a pas",
  "This preset does not support OAuth identity grants.":
    "Ce préréglage ne prend pas en charge les attributions d'identité OAuth.",
  "This preset is no longer available.": "Ce préréglage n'est plus disponible.",
  "This target is unavailable on this deployment.":
    "Cette cible n'est pas disponible sur ce déploiement.",
  "This template has no guided setup.": "Ce modèle n'a pas de configuration guidée.",
  "This trigger may have been deleted, moved to another agent, or the URL may be wrong.":
    "Ce déclencheur a peut-être été supprimé, déplacé vers un autre agent ou l'URL est peut-être erronée.",
  "This uninstalls the agent's GitHub App. GitHub triggers will stop firing.":
    "Cela désinstalle l'application GitHub de l'agent. Les déclencheurs GitHub cesseront de se déclencher.",
  Threshold: "Seuil",
  "Toggle settings": "Basculer les paramètres",
  "Tool activity shows that a tool ran, never its name or arguments. Everything is for callers who own both ends.":
    "L'activité de l'outil montre qu'un outil a été exécuté, jamais son nom ou ses arguments. Tout est destiné aux appelants qui possèdent les deux extrémités.",
  "Tool activity text": "Texte de l'activité de l'outil",
  "Tool names": "Noms des outils",
  "Tool names, arguments, and results are never sent to public clients.":
    "Les noms, arguments et résultats des outils ne sont jamais envoyés aux clients publics.",
  "Tool prefix collides with another attachment name.":
    "Le préfixe de l'outil entre en collision avec un autre nom de pièce jointe.",
  "Tool visibility": "Visibilité de l'outil",
  "Tools are listed when a chat signs in.":
    "Les outils sont répertoriés lorsqu'un chat se connecte.",
  "Tools from the harness, enabled capabilities, and agent":
    "Outils du cadre d'exécution, capacités activées et agent",
  "Tools load on demand": "Chargement des outils à la demande",
  "Top K": "Haut K",
  "Trigger not found": "Déclencheur introuvable",
  "Triggered runs have no user account; they use the organization account.":
    "Les exécutions déclenchées n'ont pas de compte utilisateur ; ils utilisent le compte de l'organisation.",
  Triggers: "Déclencheurs",
  "Turnstile secret key": "Clé secrète du tourniquet",
  "Turnstile site key": "Clé du site tourniquet",
  "Two-way sync. Edit here or in GitHub; conflicting changes pause sync.":
    "Synchronisation bidirectionnelle. Modifiez ici ou dans GitHub ; les modifications contradictoires suspendent la synchronisation.",
  "Unable to load collections.": "Impossible de charger les collections.",
  Unavailable: "Indisponible",
  "Unavailable or archived": "Indisponible ou archivé",
  "unknown error": "erreur inconnue",
  Unpublish: "Annuler la publication",
  "Unsaved draft. Save changes to update the Agent definition. Running turns continue.":
    "Brouillon non enregistré. Enregistrez les modifications pour mettre à jour la définition de l'agent. Les virages en cours continuent.",
  "Upload a logo in the App's settings, or give the agent an avatar first.":
    "Téléchargez un logo dans les paramètres de l'application ou donnez d'abord un avatar à l'agent.",
  "Upload avatar": "Télécharger un avatar",
  "Use a Sandbox Template": "Utiliser un modèle Sandbox",
  "Use a whole number up to 1,000,000.": "Utilisez un nombre entier jusqu'à 1 000 000.",
  "Use Add to Slack to try again, or configure Slack manually.":
    "Utilisez Ajouter à Slack pour réessayer ou configurez Slack manuellement.",
  "Use an amount above 0, up to 1,000,000.":
    "Utilisez un montant supérieur à 0, jusqu'à 1 000 000.",
  "Use avatar": "Utiliser un avatar",
  "Use credentials from an existing Slack app. Add to Slack fills these in for you.":
    "Utilisez les informations d'identification d'une application Slack existante. Ajouter à Slack les remplit pour vous.",
  "Use credentials from your Slack app to connect this channel. You can save now and finish setup later.":
    "Utilisez les informations d'identification de votre application Slack pour connecter cette chaîne. Vous pouvez enregistrer maintenant et terminer la configuration plus tard.",
  "Use it": "Utilisez-le",
  "Use my GitHub connection": "Utiliser ma connexion GitHub",
  "Use my GitHub connection for api.github.com and git. Modal adds the token to requests, so the agent never sees it.":
    "Utilisez ma connexion GitHub pour api.github.com et git. Modal ajoute le jeton aux requêtes afin que l'agent ne le voie jamais.",
  "Use session model": "Utiliser le modèle de session",
  "Use the shared Everruns App": "Utilisez l'application Everruns partagée",
  "Use up to": "Utiliser jusqu'à",
  "User servers of the person chatting": "Serveurs utilisateurs de la personne qui discute",
  Uses: "Utilisations",
  "Verified with constant-time comparison; sent as Authorization: Bearer <token> or X-Everruns-FCP-Token. Never echoed in responses.":
    "Vérifié avec comparaison à temps constant ; envoyé comme autorisation : Bearer <token> ou X-Everruns-FCP-Token. Jamais repris dans les réponses.",
  "View all {value0} channels": "Afficher toutes les chaînes {value0}",
  "View catalog": "Voir le catalogue",
  "View on GitHub": "Voir sur GitHub",
  "View or update the credentials for this agent’s Slack app.":
    "Afficher ou mettre à jour les informations d'identification de l'application Slack de cet agent.",
  "VM (own kernel)": "VM (propre noyau)",
  Voice: "Voix",
  "Voice calls need an OpenAI provider in this organization. Call the agent with the button below, the microphone in its chat, or the API with an organization API key. The agent writes every answer; the speech model only listens and speaks.":
    "Les appels vocaux nécessitent un fournisseur OpenAI dans cette organisation. Appelez l'agent avec le bouton ci-dessous, le microphone dans son chat, ou l'API avec une clé API d'organisation. L'agent écrit chaque réponse ; le modèle de parole écoute et parle uniquement.",
  "Waits for a finished thought": "Attend une pensée terminée",
  "Wakes itself": "Se réveille",
  "Wakes itself:": "Se réveille :",
  "Wakes on green to red and red to green for each workflow and branch, not on every run.":
    "Se réveille du vert au rouge et du rouge au vert pour chaque flux de travail et branche, pas à chaque exécution.",
  "Warm and brief. Spell out numbers digit by digit.":
    "Chaleureux et bref. Épelez les nombres chiffre par chiffre.",
  "was removed": "a été supprimé",
  "Ways in": "Voies d'entrée",
  Webhook: "Webhook",
  "Webhook token": "Jeton Webhook",
  "Welcome message": "Message de bienvenue",
  "What callers see": "Ce que voient les appelants",
  "What should this agent do?": "Que doit faire cet agent ?",
  "When idle": "En cas d'inactivité",
  "When it matches": "Quand ça correspond",
  "When off, every request must present the bearer token below. The FCP handshake (GET on the channel URL) stays public so clients can discover how to authenticate.":
    "Lorsqu'elle est désactivée, chaque demande doit présenter le jeton du porteur ci-dessous. La poignée de main FCP (GET sur l'URL du canal) reste publique afin que les clients puissent découvrir comment s'authentifier.",
  "When on, anyone with the link can chat (optionally gated by the shared token below). Turn off to require sign-in. You must enable Google sign-in below, otherwise the chat rejects every request.":
    "Lorsque cette option est activée, toute personne disposant du lien peut discuter (éventuellement fermée par le jeton partagé ci-dessous). Désactivez-la pour exiger une connexion. Vous devez activer la connexion Google ci-dessous, sinon le chat rejettera chaque demande.",
  "When the caller interrupts": "Lorsque l'appelant interrompt",
  "When the provider cannot answer": "Lorsque le fournisseur ne peut pas répondre",
  "When this agent wakes up on its own": "Lorsque cet agent se réveille tout seul",
  "Where it works": "Où ça marche",
  "Who should this server act as?": "En tant que qui ce serveur doit-il agir ?",
  "with a JSON-RPC 2.0 envelope. Only the": "avec une enveloppe JSON-RPC 2.0. Seul le",
  "With failed turns": "Avec tours ratés",
  Words: "Mots",
  "Work on": "Travailler sur",
  "Working on it": "J'y travaille",
  "Working out the plan…": "Élaboration du plan…",
  "Workspace ID": "ID de l'espace de travail",
  "Workspace path": "Chemin de l'espace de travail",
  Writable: "Écrit",
  "xoxb-...": "xoxb-...",
  "You chase overdue invoices. Each weekday morning...":
    "Vous courez après les factures en souffrance. Chaque matin de la semaine...",
  "You connect GitHub to the agent after it is created.":
    "Vous connectez GitHub à l'agent après sa création.",
  "You get a new secret for the same key. The current secret keeps working for the overlap, so you can deploy the new one first.":
    "Vous obtenez un nouveau secret pour la même clé. Le secret actuel continue de fonctionner pendant le chevauchement, vous pouvez donc d'abord déployer le nouveau.",
  "You need permission to create GitHub Apps in that organization.":
    "Vous avez besoin d'une autorisation pour créer des applications GitHub dans cette organisation.",
  "Your code calls this agent with an agent key. A key reaches only this agent's session routes and sees only the sessions it started.":
    "Votre code appelle cet agent avec une clé d'agent. Une clé atteint uniquement les itinéraires de session de cet agent et voit uniquement les sessions qu'il a démarrées.",
  "Your code sends a key as": "Votre code envoie une clé comme",
  "Your current definition stays in Everruns and remains editable. New sync attempts stop. A GitHub write already in flight may still complete.":
    "Votre définition actuelle reste dans Everruns et reste modifiable. Les nouvelles tentatives de synchronisation s'arrêtent. Une écriture GitHub déjà en cours peut encore se terminer.",
  "Your local definition is retained.": "Votre définition locale est conservée.",
  "Your server": "Votre serveur",
  "your-org": "votre organisation",
  Zoom: "Zoom",
};
