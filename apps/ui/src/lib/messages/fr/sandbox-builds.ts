export const messages: Readonly<Record<string, string>> = {
  "Build snapshot": "Créer un instantané",
  "Run an install script once, then reuse the resulting provider snapshot.":
    "Exécutez un script d’installation une fois, puis réutilisez l’instantané du fournisseur obtenu.",
  Provider: "Fournisseur",
  "Base snapshot": "Instantané de base",
  "Provider default": "Valeur par défaut du fournisseur",
  "Source revision": "Révision source",
  "Select and save a named Organization provider account before building.":
    "Sélectionnez et enregistrez un compte fournisseur nommé de l’organisation avant de lancer la création.",
  "Save your configuration changes before building.":
    "Enregistrez les modifications de configuration avant de lancer la création.",
  "Install script": "Script d’installation",
  "Verification and build access": "Vérification et accès de création",
  "Verification command (optional)": "Commande de vérification (facultatif)",
  "Metadata file (optional)": "Fichier de métadonnées (facultatif)",
  "Capture up to 16 KiB of non-secret software notes written by your script.":
    "Capturer jusqu’à 16 Kio de notes logicielles non confidentielles écrites par votre script.",
  "Total build timeout (seconds)": "Délai total de création (secondes)",
  "Build network": "Réseau de création",
  "Loading…": "Chargement…",
  "Build result": "Résultat de la création",
  Step: "Étape",
  "Resource cleanup is pending and will be retried.":
    "Le nettoyage des ressources est en attente et sera réessayé.",
  "Captured software notes": "Notes logicielles capturées",
  "Build log": "Journal de création",
  "Waiting for output…": "En attente de sortie…",
  "Save new revision": "Enregistrer une nouvelle révision",
  "Discard snapshot": "Supprimer l’instantané",
  "The source revision changed. Build again from the current template.":
    "La révision source a changé. Relancez la création depuis le modèle actuel.",
  Queued: "En attente",
  Running: "En cours",
  Ready: "Prêt",
  Failed: "Échec",
  Cancelled: "Annulé",
  Saved: "Enregistré",
  Discarded: "Supprimé",
  "Starting build compute": "Démarrage de l’environnement de création",
  "Running install script": "Exécution du script d’installation",
  "Verifying installation": "Vérification de l’installation",
  "Creating snapshot": "Création de l’instantané",
  "Verifying fresh snapshot": "Vérification d’un nouvel environnement depuis l’instantané",
  "Cancellation requested": "Annulation demandée",
  "Cancel build": "Annuler la création",
  "Built snapshot": "Instantané créé",
  "Sandbox Template sections": "Sections du modèle de sandbox",
  Configuration: "Configuration",
  "Snapshot builds currently support Daytona. Other template targets remain configurable.":
    "La création d’instantanés prend actuellement en charge Daytona. Les autres cibles de modèle restent configurables.",
  "Runs in fresh isolated build compute. Launch bootstrap commands stay separate. Do not put credentials in scripts.":
    "Exécuté dans un nouvel environnement de création isolé. Les commandes de démarrage restent distinctes. Ne placez pas d’identifiants dans les scripts.",
  "Runs after installation and again on a fresh sandbox from the snapshot. Only what this command checks is verified.":
    "Exécutée après l’installation puis dans une nouvelle sandbox issue de l’instantané. Seuls les points contrôlés par cette commande sont vérifiés.",
  "Allow package sources during installation without expanding the finished sandbox’s runtime policy.":
    "Autorisez les sources de paquets pendant l’installation sans élargir la politique d’exécution de la sandbox finale.",
  "Build compute and snapshot storage are billed to the selected provider account. Runtime credentials and private workspace data are not attached.":
    "Le calcul et le stockage des instantanés sont facturés au compte fournisseur sélectionné. Les identifiants d’exécution et les données privées de l’espace de travail ne sont pas joints.",
  "Author-supplied reference data. These notes do not grant capabilities or change runtime policy.":
    "Données de référence fournies par l’auteur. Ces notes n’accordent aucune capacité et ne modifient pas la politique d’exécution.",
  "Discard this unused provider snapshot? Applied template revisions are kept.":
    "Supprimer cet instantané fournisseur inutilisé ? Les révisions de modèle appliquées sont conservées.",
  "Older builds": "Créations précédentes",
  "Reuse script": "Réutiliser le script",
};
