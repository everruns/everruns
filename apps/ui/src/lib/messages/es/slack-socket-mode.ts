// es product copy for Slack Socket Mode: how Slack reaches a channel.
export const messages: Readonly<Record<string, string>> = {
  "Private approval cards": "Tarjetas de aprobación privadas",
  "Show approval cards only to the requester. If a card disappears, answer the pending request in Everruns. Agent replies remain visible in the conversation.":
    "Mostrar las tarjetas de aprobación solo a quien hizo la solicitud. Si una tarjeta desaparece, responda a la solicitud pendiente en Everruns. Las respuestas del agente siguen siendo visibles en la conversación.",

  "Socket Mode": "Socket Mode",
  "How Slack reaches Everruns": "Cómo llega Slack a Everruns",
  "Everruns connects out to Slack. Works without a public address, such as on your own computer.":
    "Everruns se conecta a Slack por su cuenta. Funciona sin dirección pública, por ejemplo en tu propio ordenador.",
  "Slack sends each message to this server. Needs a public HTTPS address.":
    "Slack envía cada mensaje a este servidor. Necesita una dirección HTTPS pública.",
  "App-level token": "Token de nivel de app",
  "In your Slack app, open Basic Information, then App-Level Tokens, and generate one with the connections:write scope.":
    "En tu app de Slack, abre Basic Information y luego App-Level Tokens, y genera uno con el permiso connections:write.",
  "Open the app's settings in Slack": "Abrir la configuración de la app en Slack",
  "These fields are optional when saving. Everruns connects to Slack once an app-level token is saved; a bot token is needed to send replies.":
    "Estos campos son opcionales al guardar. Everruns se conecta a Slack en cuanto se guarda un token de nivel de app; se necesita un token de bot para enviar respuestas.",
  "Create an app from this channel’s manifest in Slack and install it to your workspace. Then generate an app-level token and enter it with the bot token below.":
    "Crea una app en Slack a partir del manifiesto de este canal e instálala en tu espacio de trabajo. Después genera un token de nivel de app e introdúcelo abajo junto con el token de bot.",
  "Everruns is connected to Slack. Invite the bot to a channel, then @mention it to test.":
    "Everruns está conectado a Slack. Invita al bot a un canal y menciónalo con @ para probarlo.",
};
