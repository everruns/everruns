// fr product copy for Slack Socket Mode: how Slack reaches a channel.
export const messages: Readonly<Record<string, string>> = {
  "Socket Mode": "Socket Mode",
  "How Slack reaches Everruns": "Comment Slack joint Everruns",
  "Everruns connects out to Slack. Works without a public address, such as on your own computer.":
    "Everruns se connecte lui-même à Slack. Fonctionne sans adresse publique, par exemple sur votre propre ordinateur.",
  "Slack sends each message to this server. Needs a public HTTPS address.":
    "Slack envoie chaque message à ce serveur. Nécessite une adresse HTTPS publique.",
  "App-level token": "Jeton au niveau de l’app",
  "In your Slack app, open Basic Information, then App-Level Tokens, and generate one with the connections:write scope.":
    "Dans votre app Slack, ouvrez Basic Information, puis App-Level Tokens, et générez-en un avec la portée connections:write.",
  "Open the app's settings in Slack": "Ouvrir les paramètres de l’app dans Slack",
  "These fields are optional when saving. Everruns connects to Slack once an app-level token is saved; a bot token is needed to send replies.":
    "Ces champs sont facultatifs à l’enregistrement. Everruns se connecte à Slack dès qu’un jeton au niveau de l’app est enregistré ; un jeton de bot est nécessaire pour répondre.",
  "Create an app from this channel’s manifest in Slack and install it to your workspace. Then generate an app-level token and enter it with the bot token below.":
    "Créez une app dans Slack à partir du manifeste de ce canal et installez-la dans votre espace de travail. Générez ensuite un jeton au niveau de l’app et saisissez-le ci-dessous avec le jeton de bot.",
  "Everruns is connected to Slack. Invite the bot to a channel, then @mention it to test.":
    "Everruns est connecté à Slack. Invitez le bot dans un canal, puis mentionnez-le avec @ pour le tester.",
};
