// de product copy for Slack Socket Mode: how Slack reaches a channel.
export const messages: Readonly<Record<string, string>> = {
  "Socket Mode": "Socket Mode",
  "How Slack reaches Everruns": "Wie Slack Everruns erreicht",
  "Everruns connects out to Slack. Works without a public address, such as on your own computer.":
    "Everruns verbindet sich selbst mit Slack. Funktioniert ohne öffentliche Adresse, etwa auf Ihrem eigenen Computer.",
  "Slack sends each message to this server. Needs a public HTTPS address.":
    "Slack sendet jede Nachricht an diesen Server. Benötigt eine öffentliche HTTPS-Adresse.",
  "App-level token": "App-Level-Token",
  "In your Slack app, open Basic Information, then App-Level Tokens, and generate one with the connections:write scope.":
    "Öffnen Sie in Ihrer Slack-App „Basic Information“, dann „App-Level Tokens“, und erzeugen Sie eines mit dem Scope connections:write.",
  "Open the app's settings in Slack": "Einstellungen der App in Slack öffnen",
  "These fields are optional when saving. Everruns connects to Slack once an app-level token is saved; a bot token is needed to send replies.":
    "Diese Felder sind beim Speichern optional. Everruns verbindet sich mit Slack, sobald ein App-Level-Token gespeichert ist; für Antworten wird ein Bot-Token benötigt.",
  "Create an app from this channel’s manifest in Slack and install it to your workspace. Then generate an app-level token and enter it with the bot token below.":
    "Erstellen Sie in Slack eine App aus dem Manifest dieses Kanals und installieren Sie sie in Ihrem Workspace. Erzeugen Sie dann ein App-Level-Token und geben Sie es unten zusammen mit dem Bot-Token ein.",
  "Everruns is connected to Slack. Invite the bot to a channel, then @mention it to test.":
    "Everruns ist mit Slack verbunden. Laden Sie den Bot in einen Kanal ein und erwähnen Sie ihn mit @, um ihn zu testen.",
};
