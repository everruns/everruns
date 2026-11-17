// es product copy for tool approval: Settings → Approvals, My approvals, the agent's
// approval settings, the session mode switch and review marks in chat
// (knowledge/execution/tool-approval.md).
export const messages: Readonly<Record<string, string>> = {
  "A reviewer answers first, asks you when unsure":
    "Un revisor responde primero y te pregunta cuando duda",
  "A reviewer answers routine calls; you get the unclear ones":
    "Un revisor responde las llamadas rutinarias; las dudosas te llegan a ti",
  Action: "Acción",
  "Add a note": "Añadir una nota",
  "Add rule": "Añadir regla",
  "Agent default": "Predeterminado del agente",
  "Allow at least one mode.": "Permite al menos un modo.",
  "Allowed modes": "Modos permitidos",
  "Always ask": "Preguntar siempre",
  "Applies from the next tool call.": "Se aplica desde la siguiente llamada a herramienta.",
  "Applies to sessions you start. You can still switch one session from its header, within what the organization allows.":
    "Se aplica a las sesiones que inicias. Aún puedes cambiar una sesión desde su encabezado, dentro de lo que permite la organización.",
  "Approval mode": "Modo de aprobación",
  "Approval mode: {mode}": "Modo de aprobación: {mode}",
  "Approvals: {mode}": "Aprobaciones: {mode}",
  "Asked a person": "Se preguntó a una persona",
  "Asks before any change": "Pregunta antes de cualquier cambio",
  "Asks before risky calls": "Pregunta antes de llamadas arriesgadas",
  "Auto review is paused after several blocked calls. Every call now waits for you until you approve one.":
    "La revisión automática está en pausa tras varias llamadas bloqueadas. Cada llamada te espera ahora hasta que apruebes una.",
  "Auto shows only when the organization allows it.":
    "Auto solo aparece si la organización lo permite.",
  "Auto-allowed": "Permitido automáticamente",
  "Choose which approval modes agents may use and set rules for every agent":
    "Elige qué modos de aprobación pueden usar los agentes y define reglas para todos los agentes",
  "Could not change the mode. You may not be allowed to manage this session.":
    "No se pudo cambiar el modo. Puede que no tengas permiso para gestionar esta sesión.",
  "Could not load recent reviews.": "No se pudieron cargar las revisiones recientes.",
  "Could not save.": "No se pudo guardar.",
  "Each agent's own mode": "El modo propio de cada agente",
  "How agents in this organization get a go-ahead before they use a tool.":
    "Cómo obtienen luz verde los agentes de esta organización antes de usar una herramienta.",
  "How tool calls in your own sessions get approved.":
    "Cómo se aprueban las llamadas a herramientas en tus propias sesiones.",
  "Modes agents and people may pick. A mode you turn off is raised to the next more careful one you allow.":
    "Modos que pueden elegir agentes y personas. Un modo desactivado sube al siguiente modo permitido más prudente.",
  "My approvals": "Mis aprobaciones",
  "My extra rules": "Mis reglas adicionales",
  "Never asks": "Nunca pregunta",
  "No auto reviews yet.": "Aún no hay revisiones automáticas.",
  Normal: "Normal",
  "Not available: this agent's manager turned approvals on":
    "No disponible: el responsable de este agente activó las aprobaciones",
  "Note {number}": "Nota {number}",
  "Notes for the reviewer": "Notas para el revisor",
  "Only organization owners and admins can change these settings.":
    "Solo los propietarios y administradores de la organización pueden cambiar esta configuración.",
  "Only the prod-* clusters are production.": "Solo los clústeres prod-* son producción.",
  "Personal rules can only add caution: Always ask or Never. They apply to calls agents make on your behalf.":
    "Las reglas personales solo pueden añadir cautela: Preguntar siempre o Nunca. Se aplican a las llamadas que los agentes hacen en tu nombre.",
  "Plain sentences the auto reviewer reads for every agent: your domains, your repositories, which systems are production.":
    "Frases sencillas que el revisor automático lee para cada agente: tus dominios, tus repositorios, qué sistemas son producción.",
  "Plain sentences the auto reviewer reads, such as which repositories are yours or what counts as production.":
    "Frases sencillas que lee el revisor automático, como qué repositorios son tuyos o qué cuenta como producción.",
  Protective: "Protector",
  "Recent reviews": "Revisiones recientes",
  "Remove note {number}": "Quitar la nota {number}",
  "Remove rule for {tool}": "Quitar la regla de {tool}",
  Rules: "Reglas",
  "Rules apply in every mode; the strictest match wins. A tool name can use * as a wildcard.":
    "Las reglas se aplican en todos los modos; gana la coincidencia más estricta. Un nombre de herramienta puede usar * como comodín.",
  "Rules for every agent": "Reglas para todos los agentes",
  "Start my sessions in": "Iniciar mis sesiones en",
  "The approval mode your sessions start in and your own extra rules":
    "El modo de aprobación con el que empiezan tus sesiones y tus propias reglas adicionales",
  "The last calls the auto reviewer decided on, newest first.":
    "Las últimas llamadas sobre las que decidió el revisor automático, las más recientes primero.",
  "These apply to every agent in the organization, in every mode. When rules disagree, the most cautious one wins.":
    "Se aplican a todos los agentes de la organización, en todos los modos. Cuando las reglas discrepan, gana la más prudente.",
  "Tool name or pattern": "Nombre de herramienta o patrón",
  "Trust notes for the reviewer": "Notas de confianza para el revisor",
  "Unable to load approval settings.": "No se pudo cargar la configuración de aprobaciones.",
  Verdict: "Veredicto",
  When: "Cuándo",
  "Why you're asked:": "Por qué se te pregunta:",
  "Your own default, else the agent's mode ({mode})":
    "Tu propio predeterminado; si no, el modo del agente ({mode})",
  "github.com/acme is ours; pushing branches there is routine.":
    "github.com/acme es nuestro; subir ramas allí es rutina.",
  "{mode} (agent default)": "{mode} (predeterminado del agente)",
};
