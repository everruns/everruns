// es product copy for the agent's Channels and triggers tab, Pause agent and the GitHub page.
export const messages: Readonly<Record<string, string>> = {
  "Channels and triggers": "Canales y disparadores",
  "How people and software reach this agent, and when it starts on its own.":
    "Cómo llegan las personas y el software a este agente, y cuándo empieza por sí solo.",
  "When this agent starts on its own: schedules, webhooks, GitHub events and MCP server events.":
    "Cuándo empieza este agente por sí solo: programaciones, webhooks, eventos de GitHub y eventos de servidores MCP.",
  "Add a schedule, a webhook or an event to start this agent on its own.":
    "Añade una programación, un webhook o un evento para que este agente empiece por sí solo.",
  "Delete this trigger? It stops starting the agent.":
    "¿Eliminar este disparador? Dejará de iniciar el agente.",
  "Turn off": "Desactivar",
  "Turn on": "Activar",
  "Pause agent": "Pausar agente",
  "Resume agent": "Reanudar agente",
  "Pause {value0}? Its channels stop accepting messages and its triggers and schedules stop starting it until you resume it. Sessions already running continue.":
    "¿Pausar {value0}? Sus canales dejan de aceptar mensajes y sus disparadores y programaciones dejan de iniciarlo hasta que lo reanudes. Las sesiones en curso continúan.",
  "This agent is paused": "Este agente está en pausa",
  "Its channels do not accept messages and its triggers and schedules do not start it. Each channel and trigger keeps its own setting for when you resume.":
    "Sus canales no aceptan mensajes y sus disparadores y programaciones no lo inician. Cada canal y disparador conserva su propio ajuste para cuando lo reanudes.",
  "Publish opens this channel to callers and Unpublish closes it. Pausing the agent overrides both.":
    "Publicar abre este canal a quienes llaman y Despublicar lo cierra. Pausar el agente prevalece sobre ambos.",
  "Add a channel from an agent's Channels and triggers tab. New channels start as drafts and take no traffic until you publish them.":
    "Añade un canal desde la pestaña Canales y disparadores de un agente. Los canales nuevos empiezan como borradores y no reciben tráfico hasta que los publicas.",
  "Need another system to start the agent with an HTTP request?":
    "¿Necesitas que otro sistema inicie el agente con una solicitud HTTP?",
  "Add a webhook trigger": "Añadir un disparador webhook",
  "Trigger type": "Tipo de disparador",
  Token: "Token",
  Generate: "Generar",
  "Leave empty to keep the current token": "Déjalo vacío para conservar el token actual",
  "Copy it now: it is not shown again. Callers send it as Authorization: Bearer <token> or X-Everruns-Webhook-Token.":
    "Cópialo ahora: no se vuelve a mostrar. Quienes llaman lo envían como Authorization: Bearer <token> o X-Everruns-Webhook-Token.",
  "Recent runs": "Ejecuciones recientes",
  "Loading runs…": "Cargando ejecuciones…",
  "Starts when {value0} sends {value1}.": "Empieza cuando {value0} envía {value1}.",
  "MCP event triggers are created and changed through the API and CLI.":
    "Los disparadores de eventos MCP se crean y cambian mediante la API y la CLI.",
  "Who {value0} is on GitHub when it reads code and comments on pull requests.":
    "Quién es {value0} en GitHub cuando lee código y comenta pull requests.",
  "Not connected": "No conectado",
  "Not installed": "No instalado",
  "Own GitHub App": "App de GitHub propia",
  "Shared Everruns App": "App compartida de Everruns",
  Kind: "Tipo",
  trigger: "disparador",
  "The agent's GitHub tools, while the GitHub capability is on":
    "Las herramientas de GitHub del agente, mientras la capacidad de GitHub esté activada",
  "It may have been deleted, or you may not have access to it.":
    "Puede que se haya eliminado o que no tengas acceso.",
};
